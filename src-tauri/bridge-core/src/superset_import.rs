//! Superset (now Paradev) into Bridge (#1060): its workspaces as Bridge
//! workspaces adopted in place ([`crate::import_worktrees`]), its chats and
//! terminal agent sessions as chats, and the agent configurations you made
//! as inert setup.
//!
//! Superset runs Claude Code, Codex, OpenCode and other agents, in chats and
//! in terminals, and records the session each ran as
//! ([`store::NativeSession`]). When that session's file is in Bridge's folders
//! for the agent, the chat is read by the agent's own reader, so it is the
//! same chat the agent's import makes. When it is gone, a chat is Superset's
//! own copy, keeping the native identity.
//!
//! Built from Superset's source (`superset-sh/superset@17bbe650`) and its
//! released 2.0.11 host service, run headless in a sandbox; the schemas are
//! rebuilt with its own migrations (`testing/fixtures/import/superset/`).
//! Only databases are opened, read-only, and only named columns of allowed
//! tables; never [`store::DENIED_FILES`] or an agent configuration's values.

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
    BundleChat, BundleEntry, BundleEntryKind, BundleSetup, BundleSource, BundleTool, EntriesSource, NativeRef, TitleSource,
    ToolStatus, TransferBundle,
};
use crate::import_worktrees::Candidate;
use crate::BridgeError;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use store::{AgentConfig, Body, Chat, Host, Item, Kind, NativeSession};

/// The import id the Import page and the job use.
pub const PROVIDER: &str = "superset";
pub const ADAPTER_VERSION: &str = "1";
pub const STORE_GATE: &str = "superset-sqlite";

#[derive(Debug, Default, Clone, Copy)]
pub struct SupersetImporter;

/// Superset's app homes on this Mac: `$SUPERSET_HOME_DIR` (or
/// `$PARADEV_HOME_DIR`), else `~/.paradev` and the older `~/.superset`,
/// each read once however they link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub homes: Vec<PathBuf>,
}

/// One organization's databases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgPaths {
    pub id: String,
    pub host: PathBuf,
    pub chat: Option<PathBuf>,
}

impl Paths {
    pub fn at(home: &Path) -> Self {
        Self { homes: vec![home.to_path_buf()] }
    }

    /// For a home folder, honouring the overrides as Superset does: the
    /// `SUPERSET_` name wins over the `PARADEV_` one.
    pub fn for_home(home: &Path, superset_home: Option<&Path>, paradev_home: Option<&Path>) -> Self {
        if let Some(dir) = superset_home.or(paradev_home) {
            return Self::at(dir);
        }
        let mut seen = HashSet::new();
        let homes = [home.join(".paradev"), home.join(".superset")]
            .into_iter()
            .filter(|dir| dir.is_dir())
            .filter(|dir| seen.insert(dir.canonicalize().unwrap_or_else(|_| dir.clone())))
            .collect();
        Self { homes }
    }

    /// This machine's, from the environment.
    pub fn from_env(home: &Path) -> Self {
        let var = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty()).map(PathBuf::from);
        Self::for_home(home, var("SUPERSET_HOME_DIR").as_deref(), var("PARADEV_HOME_DIR").as_deref())
    }

    /// Every organization with a host database, once each: the first home
    /// that has it wins.
    pub fn organizations(&self) -> Vec<OrgPaths> {
        let mut out: BTreeMap<String, OrgPaths> = BTreeMap::new();
        for home in &self.homes {
            let mut found: Vec<PathBuf> = std::fs::read_dir(home.join("host")).into_iter().flatten().flatten().map(|entry| entry.path()).collect();
            found.sort();
            for folder in found {
                let Some(id) = folder.file_name().and_then(|name| name.to_str()).and_then(store::plain_id) else { continue };
                let host = folder.join(store::HOST_DB);
                if !host.is_file() || out.contains_key(&id) {
                    continue;
                }
                let chat = Some(folder.join(store::CHAT_DB)).filter(|path| path.is_file());
                out.insert(id.clone(), OrgPaths { id, host, chat });
            }
        }
        out.into_values().collect()
    }

    pub fn installed(&self) -> bool {
        !self.organizations().is_empty()
    }
}

/// One organization's data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Organization {
    pub id: String,
    pub host_path: PathBuf,
    pub chat_path: Option<PathBuf>,
    pub host: Host,
    pub chat_version: Option<i64>,
    pub chats: Vec<Chat>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Store {
    pub orgs: Vec<Organization>,
}

fn read_org(org: &OrgPaths) -> Result<Organization, String> {
    let conn = crate::import_sqlite::open(&org.host).map_err(|error| format!("Bridge could not open Superset's workspaces: {error}"))?;
    let host = store::read_host(&conn)?;
    let (chat_version, chats) = match &org.chat {
        Some(path) => {
            let conn = crate::import_sqlite::open(path).map_err(|error| format!("Bridge could not open Superset's chats: {error}"))?;
            let (version, chats) = store::read_chats(&conn)?;
            (Some(version), chats)
        }
        None => (None, Vec::new()),
    };
    Ok(Organization { id: org.id.clone(), host_path: org.host.clone(), chat_path: org.chat.clone(), host, chat_version, chats })
}

/// Read every organization, or say why not.
pub fn read(paths: &Paths) -> Result<Store, String> {
    let orgs = paths.organizations();
    if orgs.is_empty() {
        return Err("Superset was not found on this Mac".into());
    }
    Ok(Store { orgs: orgs.iter().map(read_org).collect::<Result<_, _>>()? })
}

/// The workspace a chat or terminal ran in, unless its project was deleted.
fn workspace<'a>(org: &'a Organization, id: &str) -> Result<Option<&'a store::Workspace>, ()> {
    let Some(workspace) = org.host.workspaces.get(id) else { return Ok(None) };
    let deleted = workspace.project.as_deref().and_then(|project| org.host.projects.get(project)).is_some_and(|project| project.deleted);
    if deleted { Err(()) } else { Ok(Some(workspace)) }
}

/// A chat Bridge can bring over: an agent Bridge runs, not in a deleted project.
pub fn is_live(org: &Organization, chat: &Chat) -> bool {
    store::harness_of_chat(&chat.harness).is_some() && workspace(org, &chat.workspace).is_ok()
}

/// A terminal session Bridge can bring over: an agent Bridge runs that
/// reported its session, not in a deleted project.
pub fn binding_is_live(org: &Organization, binding: &store::Binding) -> bool {
    binding.native.is_some() && workspace(org, &binding.workspace).is_ok()
}

/// The folder a chat or terminal ran in.
pub fn folder(org: &Organization, workspace_id: &str) -> Option<PathBuf> {
    workspace(org, workspace_id).ok().flatten().map(|workspace| workspace.path.clone())
}

pub fn is_archived(org: &Organization, workspace_id: &str) -> bool {
    workspace(org, workspace_id).ok().flatten().is_some_and(|workspace| workspace.archived)
}

/// Every live chat, with its organization.
pub fn chats(store: &Store) -> impl Iterator<Item = (&Organization, &Chat)> {
    store.orgs.iter().flat_map(|org| org.chats.iter().filter(move |chat| is_live(org, chat)).map(move |chat| (org, chat)))
}

/// Every live terminal session whose session no live chat already names.
pub fn bindings(store: &Store) -> impl Iterator<Item = (&Organization, &store::Binding)> {
    let in_chats: HashSet<NativeSession> = chats(store).filter_map(|(_, chat)| chat.native.clone()).collect();
    store
        .orgs
        .iter()
        .flat_map(|org| org.host.bindings.iter().filter(move |binding| binding_is_live(org, binding)).map(move |binding| (org, binding)))
        .filter(move |(_, binding)| binding.native.as_ref().is_some_and(|native| !in_chats.contains(native)))
}

/// Every worktree workspace, once each, named by Superset's name. Archived
/// when Superset archived it. Main checkouts and session folders are not.
pub fn worktree_candidates(store: &Store) -> Vec<Candidate> {
    let mut by_path: BTreeMap<PathBuf, Candidate> = BTreeMap::new();
    for org in &store.orgs {
        for workspace in org.host.workspaces.values().filter(|workspace| workspace.kind == Kind::Worktree) {
            let Ok(Some(_)) = self::workspace(org, &workspace.id) else { continue };
            let project = workspace.project.as_deref().and_then(|project| org.host.projects.get(project));
            // Before workspaces had a type (0017), the project's own checkout
            // was one too: it is a project, not a workspace.
            if project.is_some_and(|project| project.root == workspace.path) {
                continue;
            }
            let entry = by_path.entry(workspace.path.clone()).or_insert_with(|| Candidate {
                tool: PROVIDER,
                name: workspace.path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default(),
                title: workspace.name.clone().filter(|name| !name.trim().is_empty()),
                archived: true,
                source_ref: format!("superset:{}", workspace.id),
                scripts: project.and_then(|project| store::project_scripts(&project.root)),
                path: workspace.path.clone(),
            });
            entry.archived &= workspace.archived;
        }
    }
    by_path.into_values().collect()
}

/// What detection shows for Superset.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counts {
    pub chats: u64,
    pub projects: u64,
    pub workspaces: u64,
}

pub fn counts(store: &Store) -> Counts {
    let projects: HashSet<&Path> = store.orgs.iter().flat_map(|org| org.host.projects.values()).filter(|project| !project.deleted).map(|project| project.root.as_path()).collect();
    let workspaces = worktree_candidates(store).iter().filter(|candidate| !candidate.archived).count();
    Counts { chats: (chats(store).count() + bindings(store).count()) as u64, projects: projects.len() as u64, workspaces: workspaces as u64 }
}

/// How the live chats and terminal sessions come over.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatPlan {
    /// Read by the agent's own importer, from these files.
    pub native: BTreeSet<NativeSession>,
    /// Superset's own copy, by `(organization, session id)`.
    pub copies: Vec<(String, String)>,
    /// Terminal sessions whose transcript is gone: Superset keeps no copy.
    pub gone: usize,
}

pub fn plan_chats(store: &Store, found: &HashSet<NativeSession>) -> ChatPlan {
    let mut plan = ChatPlan::default();
    for (org, chat) in chats(store) {
        match chat.native.as_ref().filter(|native| found.contains(*native)) {
            Some(native) => {
                plan.native.insert(native.clone());
            }
            None => plan.copies.push((org.id.clone(), chat.id.clone())),
        }
    }
    for (_, binding) in bindings(store) {
        match binding.native.as_ref().filter(|native| found.contains(*native)) {
            Some(native) => {
                plan.native.insert(native.clone());
            }
            None => plan.gone += 1,
        }
    }
    plan
}

/// Every native session the live chats and terminals name.
pub fn wanted(store: &Store) -> BTreeSet<NativeSession> {
    chats(store).filter_map(|(_, chat)| chat.native.clone()).chain(bindings(store).filter_map(|(_, binding)| binding.native.clone())).collect()
}

/// Folders the agents' importers should also read: the Claude Code folder a
/// terminal's transcript is in, and Superset's default Claude Code folder and
/// Codex home.
pub fn extra_roots(store: &Store) -> Vec<(&'static str, PathBuf)> {
    let mut out = BTreeSet::new();
    for org in &store.orgs {
        for transcript in org.host.bindings.iter().filter(|binding| binding.agent == "claude").filter_map(|binding| binding.transcript.as_deref()) {
            // `<root>/projects/<folder>/<session>.jsonl`.
            let root = transcript.parent().and_then(Path::parent).filter(|projects| projects.file_name().is_some_and(|name| name == "projects")).and_then(Path::parent);
            if let Some(root) = root {
                out.insert(("claude", root.to_path_buf()));
            }
        }
        if let Some(root) = &org.host.default_claude {
            out.insert(("claude", root.clone()));
        }
        if let Some(root) = &org.host.default_codex {
            out.insert(("codex", root.clone()));
        }
    }
    out.into_iter().collect()
}

/// Agent configurations you made: a custom agent, or one you gave an
/// environment. Superset's seeded built-ins are not.
pub fn own_configs(store: &Store) -> impl Iterator<Item = (&Organization, &AgentConfig)> {
    store
        .orgs
        .iter()
        .flat_map(|org| org.host.configs.iter().map(move |config| (org, config)))
        .filter(|(_, config)| !config.env_names.is_empty() || config.preset.as_deref().is_none_or(|preset| preset.starts_with("custom")))
}

/// `<database>#<id>`: a chat in `chat.db`, or `agent-config:<id>` in `host.db`.
pub fn source_ref(database: &Path, id: &str) -> String {
    format!("{}#{id}", database.to_string_lossy())
}

pub fn parse_source_ref(source_ref: &str) -> Option<(PathBuf, String)> {
    let (database, id) = source_ref.rsplit_once('#')?;
    (!database.is_empty() && !id.is_empty()).then(|| (PathBuf::from(database), id.to_owned()))
}

const CONFIG_PREFIX: &str = "agent-config:";

fn supported_format_versions() -> BTreeMap<String, String> {
    BTreeMap::from([(STORE_GATE.to_owned(), STORE_GATE.to_owned())])
}

fn artifact(reference: String, label: String, kind: CandidateKind, title: Option<String>, folder: Option<PathBuf>, at: Option<String>) -> DiscoveredArtifact {
    DiscoveredArtifact {
        artifact_id: deterministic_identity(PROVIDER, &reference, Some(&label), at.as_deref().unwrap_or_default(), &kind),
        canonical_source_ref: reference,
        source_label: label,
        kind,
        classification: SourceClassification::VersionGatedPrivate,
        stability: Stability::VersionGated,
        estimated_bytes: 4096,
        modified_at: at.clone(),
        required_schema_gate: Some(STORE_GATE.into()),
        title,
        project_path: folder.map(|path| path.to_string_lossy().into_owned()),
        started_at: at,
    }
}

/// The job's work item for one copied chat.
pub fn copy_artifact(org: &Organization, chat: &Chat) -> Option<DiscoveredArtifact> {
    let path = org.chat_path.as_ref()?;
    Some(artifact(source_ref(path, &chat.id), format!("Superset/{}", chat.id), CandidateKind::Conversation, chat.title.clone(), folder(org, &chat.workspace), chat.updated_at.clone()))
}

/// The job's work item for one agent configuration.
pub fn config_artifact(org: &Organization, config: &AgentConfig) -> DiscoveredArtifact {
    artifact(
        source_ref(&org.host_path, &format!("{CONFIG_PREFIX}{}", config.id)),
        format!("Superset/agent/{}", config.label),
        CandidateKind::ProviderInstance,
        Some(config.label.clone()),
        None,
        None,
    )
}

/// A discovery holding one batch of work, for preview and commit.
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

/// Superset's copy of a chat: its conversation, title, archived state,
/// folder, and the native identity it ran as.
pub fn copy_chat(org: &Organization, chat: &Chat, items: &[Item], reference: &str) -> BundleChat {
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
    let harness = chat.native.as_ref().map(|native| native.harness).or_else(|| store::harness_of_chat(&chat.harness)).unwrap_or("other");
    BundleChat {
        key: chat.id.clone(),
        workspace_key: None,
        cwd: folder(org, &chat.workspace).map(|path| path.to_string_lossy().into_owned()),
        title: chat.title.clone(),
        title_source: TitleSource::Tool,
        created_at: items.first().and_then(|item| item.at.clone()).or_else(|| chat.updated_at.clone()),
        updated_at: chat.updated_at.clone(),
        archived: is_archived(org, &chat.workspace),
        pinned: false,
        harness: harness.to_owned(),
        model: None,
        effort: None,
        native_ref: chat.native.as_ref().map(|native| NativeRef {
            harness: native.harness.to_owned(),
            session_id: native.id.clone(),
            config_root: None,
            transcript_path: None,
        }),
        parent_chat_key: None,
        parent_native_key: None,
        agent_role: None,
        entries_source: EntriesSource::ToolCopy,
        entries,
        pull_requests: Vec::new(),
        legacy_native_id: None,
        legacy_content_hash: None,
        source_ref: reference.to_owned(),
        source_label: format!("Superset/{}", chat.title.as_deref().unwrap_or(&chat.id)),
    }
}

/// An agent configuration as an inert setup item: never its values.
pub fn config_setup(config: &AgentConfig, reference: &str) -> BundleSetup {
    BundleSetup {
        key: config.id.clone(),
        kind: CandidateKind::ProviderInstance,
        scope: "global".into(),
        project_key: None,
        name: config.label.clone(),
        body: None,
        config: Some(json!({
            "agent": config.preset,
            "command": config.command,
            "args": config.args,
            "envNames": config.env_names,
        })),
        source_ref: reference.to_owned(),
        source_label: format!("Superset/agent/{}", config.label),
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

/// The organization a database reference names, if it is under an approved
/// root: `<home>/host/<organization>/host.db` or `chat.db`.
fn approved_org(approved: &[PathBuf], database: &Path) -> Result<OrgPaths, String> {
    let real = database.canonicalize().map_err(|error| format!("Superset's data is unavailable: {error}"))?;
    let named = real.file_name().is_some_and(|name| name == store::HOST_DB || name == store::CHAT_DB);
    if !named || !approved.iter().filter_map(|root| root.canonicalize().ok()).any(|root| real.starts_with(root)) {
        return Err("That Superset database is outside the approved folders".into());
    }
    let folder = real.parent().ok_or_else(|| "Superset's data is unavailable".to_owned())?;
    let id = folder.file_name().and_then(|name| name.to_str()).and_then(store::plain_id).ok_or_else(|| "Not a Superset organization".to_owned())?;
    let host = folder.join(store::HOST_DB);
    let chat = Some(folder.join(store::CHAT_DB)).filter(|path| path.is_file());
    Ok(OrgPaths { id, host, chat })
}

/// Every selected chat's copy and agent configuration, read into one bundle
/// and lowered to candidates. Each organization is read once.
fn preview_items(approved: &[PathBuf], artifacts: &[&DiscoveredArtifact], discovery: &DiscoveryResult) -> Result<Vec<ImportCandidate>, BridgeError> {
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
    let mut orgs: HashMap<PathBuf, Result<(Organization, Option<Connection>), String>> = HashMap::new();
    for artifact in artifacts {
        let outcome = parse_source_ref(&artifact.canonical_source_ref)
            .ok_or_else(|| "Not a Superset reference".to_owned())
            .and_then(|(database, id)| {
                let paths = approved_org(approved, &database)?;
                let entry = orgs.entry(paths.host.clone()).or_insert_with(|| {
                    let org = read_org(&paths)?;
                    let conn = match &org.chat_path {
                        Some(path) => Some(crate::import_sqlite::open(path).map_err(|error| format!("Bridge could not open Superset's chats: {error}"))?),
                        None => None,
                    };
                    Ok((org, conn))
                });
                let (org, conn) = entry.as_ref().map_err(Clone::clone)?;
                if let Some(config) = id.strip_prefix(CONFIG_PREFIX) {
                    let config = org.host.configs.iter().find(|candidate| candidate.id == config).ok_or_else(|| "Superset no longer has this agent".to_owned())?;
                    return Ok(Err(config_setup(config, &artifact.canonical_source_ref)));
                }
                let chat = org.chats.iter().find(|chat| chat.id == id && is_live(org, chat)).ok_or_else(|| "Superset no longer has this chat".to_owned())?;
                let items = match (conn, &chat.epoch) {
                    (Some(conn), Some(epoch)) => store::items(conn, &chat.id, epoch)?,
                    _ => Vec::new(),
                };
                Ok(Ok(copy_chat(org, chat, &items, &artifact.canonical_source_ref)))
            });
        match outcome {
            Ok(Ok(chat)) if chat.entries.is_empty() => out.push(unsupported_candidate(artifact, discovery, "1 chat: no messages Bridge can show")),
            Ok(Ok(chat)) => bundle.chats.push(chat),
            Ok(Err(setup)) => bundle.setup.push(setup),
            Err(reason) => out.push(unsupported_candidate(artifact, discovery, &reason)),
        }
    }
    let lowered = crate::import_bundle::lower(&crate::import_bundle::resolve(vec![bundle])?);
    out.extend(lowered.candidates);
    Ok(out)
}

impl ExternalHarnessImporter for SupersetImporter {
    fn discover(&self, request: &DiscoveryRequest) -> Result<DiscoveryResult, BridgeError> {
        if request.provider != PROVIDER {
            return Err(BridgeError::Invalid(format!("Superset importer cannot discover provider '{}'", request.provider)));
        }
        let paths = Paths { homes: request.approved_roots.iter().map(PathBuf::from).collect() };
        let store = read(&paths).map_err(BridgeError::Invalid)?;
        let mut artifacts: Vec<DiscoveredArtifact> = chats(&store).filter_map(|(org, chat)| copy_artifact(org, chat)).collect();
        artifacts.extend(own_configs(&store).map(|(org, config)| config_artifact(org, config)));
        Ok(job_discovery(&paths.homes, artifacts))
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
            .filter(|artifact| selected.contains(artifact.artifact_id.as_str()))
            .filter(|artifact| matches!(artifact.kind, CandidateKind::Conversation | CandidateKind::ProviderInstance))
            .collect();
        preview_items(&approved, &artifacts, discovery)
    }

    fn validate(&self, candidate: &ImportCandidate, schema_gate: &SchemaGate) -> ValidationResult {
        if candidate.source.provider != PROVIDER {
            return ValidationResult {
                accepted: false,
                diagnostics: vec![ImportDiagnostic {
                    code: "provider_mismatch".into(),
                    severity: "error".into(),
                    classification: SourceClassification::UnsupportedPrivate,
                    message: "Candidate does not belong to Superset".into(),
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
            return Err(BridgeError::Invalid("Superset candidate is not safe to normalize".into()));
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
                source: "Superset app home (host/<organization>/host.db: projects, workspaces, terminal_agent_bindings, host_agent_configs; chat.db: chat_sessions_local, chat_journal)".into(),
                classification: SourceClassification::VersionGatedPrivate,
                versions: vec![STORE_GATE.into()],
            }],
        }
    }
}

/// What applying Superset's titles and archived state changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Overlay {
    pub titled: usize,
    pub archived: usize,
}

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

/// Give chats that came over through their agent's import Superset's title
/// (unless renamed in Bridge since) and archived state. Never unarchives.
pub fn overlay(db: &Connection, store: &Store) -> Result<Overlay, BridgeError> {
    let mut out = Overlay::default();
    let transaction = db.unchecked_transaction()?;
    let units = chats(store)
        .filter_map(|(org, chat)| chat.native.as_ref().map(|native| (org, native, chat.title.as_deref(), chat.workspace.as_str())))
        .chain(bindings(store).filter_map(|(org, binding)| binding.native.as_ref().map(|native| (org, native, None, binding.workspace.as_str()))));
    for (org, native, title, workspace) in units {
        let Some(session) = session_for(&transaction, &native.key())? else { continue };
        if let Some(title) = title.filter(|title| !title.is_empty()) {
            out.titled += transaction.execute(
                "UPDATE sessions SET title=?1 WHERE id=?2 AND kind='imported' AND title_source='imported' AND COALESCE(title,'') <> ?1",
                params![title, session],
            )?;
        }
        if is_archived(org, workspace) {
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
#[path = "superset_import/tests.rs"]
mod tests;

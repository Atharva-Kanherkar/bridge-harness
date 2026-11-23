//! OpenCode chats into Bridge (#1055): the sessions in an OpenCode store
//! (`$XDG_DATA_HOME/opencode/opencode.db`, else
//! `~/.local/share/opencode/opencode.db`), read by [`store`] into the
//! transfer bundle and written by the same pipeline as Claude Code's and
//! Codex's.
//!
//! A session is a row, not a file, so its source reference is the store file
//! and the session id: `<store>#<session id>`. Only the session, message and
//! part tables are read; the store's account and credential tables, and
//! `auth.json`, never are. Memories and setup stay with OpenCode.

pub mod store;

#[cfg(test)]
pub(crate) mod fixture;

use crate::external_import::{
    content_hash, deterministic_identity, source_now, validate_schema_version, CandidateKind, DiscoveredArtifact,
    DiscoveryRequest, DiscoveryResult, DiscoverySelection, ExternalHarnessImporter, FixtureFormat, FixtureManifest,
    ImportCandidate, ImportDiagnostic, NormalizedImportCandidate, RedactionSummary, SchemaGate, SourceClassification,
    Stability, ValidationResult,
};
use crate::BridgeError;
use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The bundle tool id, as detection and the Import page name it.
pub const PROVIDER: &str = "opencode";
pub const ADAPTER_VERSION: &str = "1";
pub const STORE_GATE: &str = "opencode-sqlite-v1";
/// The store's file name inside OpenCode's data folder.
pub const STORE_FILE: &str = "opencode.db";

#[derive(Debug, Default, Clone, Copy)]
pub struct OpenCodeImporter;

fn supported_format_versions() -> BTreeMap<String, String> {
    BTreeMap::from([(STORE_GATE.to_owned(), STORE_GATE.to_owned())])
}

/// OpenCode's data folder for a data home: `<data home>/opencode`.
pub fn data_folder(data_home: &Path) -> PathBuf {
    data_home.join("opencode")
}

/// The data folder this daemon's OpenCode uses, when it holds a store:
/// `$XDG_DATA_HOME/opencode`, else `~/.local/share/opencode`.
pub fn machine_opencode_homes() -> Vec<PathBuf> {
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    data_home
        .map(|home| data_folder(&home))
        .and_then(|folder| folder.canonicalize().ok())
        .filter(|folder| folder.join(STORE_FILE).is_file())
        .into_iter()
        .collect()
}

/// `<store>#<session id>`.
pub fn source_ref(store: &Path, session: &str) -> String {
    format!("{}#{session}", store.to_string_lossy())
}

/// The store and session a source reference names.
pub fn parse_source_ref(source_ref: &str) -> Option<(PathBuf, String)> {
    let (store, session) = source_ref.rsplit_once('#')?;
    (!store.is_empty() && !session.is_empty()).then(|| (PathBuf::from(store), session.to_owned()))
}

/// A session OpenCode ran for a background job: in the OS temp folder, or in
/// Bridge's own title scratch folder (`<Bridge data>/chats/title-runs`).
pub fn is_background(directory: &str) -> bool {
    crate::codex_import::is_temp_folder(directory)
        || directory.trim_end_matches('/').ends_with("/chats/title-runs")
        || directory.contains("/chats/title-runs/")
}

/// The data folders under the approved roots that hold a store: the root
/// itself, or its `opencode` folder (a data home).
fn stores_under(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut stores: Vec<PathBuf> = roots
        .iter()
        .flat_map(|root| [root.join(STORE_FILE), root.join("opencode").join(STORE_FILE)])
        .filter(|store| store.is_file())
        .filter_map(|store| store.canonicalize().ok())
        .collect();
    stores.sort();
    stores.dedup();
    stores
}

fn millis_text(ms: i64) -> Option<String> {
    (ms > 0)
        .then(|| DateTime::from_timestamp_millis(ms).map(|at| at.to_rfc3339_opts(SecondsFormat::Millis, true)))
        .flatten()
}

fn artifact_for(store: &Path, row: &store::SessionRow) -> DiscoveredArtifact {
    let reference = source_ref(store, &row.id);
    let label = format!("{STORE_FILE}/{}", row.id);
    DiscoveredArtifact {
        artifact_id: deterministic_identity(
            PROVIDER,
            &reference,
            Some(&label),
            &row.updated_ms.to_string(),
            &CandidateKind::Conversation,
        ),
        canonical_source_ref: reference,
        source_label: label,
        kind: CandidateKind::Conversation,
        classification: SourceClassification::VersionGatedPrivate,
        stability: Stability::VersionGated,
        estimated_bytes: row.bytes,
        modified_at: millis_text(row.updated_ms),
        required_schema_gate: Some(STORE_GATE.into()),
        title: row.title.clone(),
        project_path: (!row.directory.is_empty()).then(|| row.directory.clone()),
        started_at: millis_text(row.created_ms),
    }
}

fn read_store(store: &Path) -> Result<Vec<store::SessionRow>, BridgeError> {
    let conn = store::open(store).map_err(|error| BridgeError::Invalid(format!("OpenCode's store could not be opened: {error}")))?;
    store::list_sessions(&conn).map_err(|error| BridgeError::Invalid(format!("OpenCode's store could not be read: {error}")))
}

impl OpenCodeImporter {
    pub fn discover_in(&self, request: &DiscoveryRequest) -> Result<DiscoveryResult, BridgeError> {
        if request.provider != PROVIDER {
            return Err(BridgeError::Invalid(format!("OpenCode importer cannot discover provider '{}'", request.provider)));
        }
        let roots = canonical_roots(&request.approved_roots)?;
        if roots.is_empty() {
            return Err(BridgeError::Invalid("Choose an OpenCode folder before discovery".into()));
        }
        let mut artifacts = Vec::new();
        for store in stores_under(&roots) {
            artifacts.extend(read_store(&store)?.iter().map(|row| artifact_for(&store, row)));
        }
        artifacts.sort_by_cached_key(job_key);
        Ok(discovery(&roots, artifacts, request.source_version.clone()))
    }
}

fn canonical_roots(roots: &[String]) -> Result<Vec<PathBuf>, BridgeError> {
    let mut out = Vec::new();
    for root in roots {
        let path = Path::new(root)
            .canonicalize()
            .map_err(|error| BridgeError::Invalid(format!("Approved OpenCode folder is unavailable: {error}")))?;
        if !path.is_dir() {
            return Err(BridgeError::Invalid("Approved OpenCode folders must be directories".into()));
        }
        out.push(path);
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn discovery(roots: &[PathBuf], artifacts: Vec<DiscoveredArtifact>, source_version: Option<String>) -> DiscoveryResult {
    DiscoveryResult {
        discovery_id: format!("disc_{}", uuid::Uuid::new_v4().simple()),
        provider: PROVIDER.into(),
        approved_roots: roots.iter().map(|root| root.to_string_lossy().into_owned()).collect(),
        source_version,
        format_versions: supported_format_versions(),
        artifacts,
        diagnostics: Vec::new(),
        discovered_at: Utc::now().to_rfc3339(),
    }
}

impl ExternalHarnessImporter for OpenCodeImporter {
    fn discover(&self, request: &DiscoveryRequest) -> Result<DiscoveryResult, BridgeError> {
        self.discover_in(request)
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
        preview_sessions(&approved, &artifacts, discovery)
    }

    fn validate(&self, candidate: &ImportCandidate, schema_gate: &SchemaGate) -> ValidationResult {
        if candidate.source.provider != PROVIDER {
            return ValidationResult {
                accepted: false,
                diagnostics: vec![unsupported_diagnostic("provider_mismatch", "Candidate does not belong to OpenCode", None)],
            };
        }
        if matches!(candidate.stability, Stability::VersionGated) {
            return validate_schema_version(schema_gate, candidate.source.schema_version.as_deref());
        }
        ValidationResult { accepted: candidate.redaction_summary.safely_representable, diagnostics: candidate.diagnostics.clone() }
    }

    fn normalize(&self, candidate: ImportCandidate) -> Result<NormalizedImportCandidate, BridgeError> {
        if candidate.source.provider != PROVIDER || !candidate.redaction_summary.safely_representable {
            return Err(BridgeError::Invalid("OpenCode candidate is not safe to normalize".into()));
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
                source: "OpenCode store (opencode.db: session_v2/session_message, session/message/part)".into(),
                classification: SourceClassification::VersionGatedPrivate,
                versions: vec![STORE_GATE.into()],
            }],
        }
    }
}

/// The store a reference names, if it is under an approved root.
fn approved_store(approved: &[PathBuf], store: &Path) -> Result<PathBuf, BridgeError> {
    let real = store
        .canonicalize()
        .map_err(|error| BridgeError::Invalid(format!("OpenCode's store is unavailable: {error}")))?;
    if approved.iter().any(|root| real.starts_with(root)) {
        Ok(real)
    } else {
        Err(BridgeError::Invalid("That OpenCode store is outside the approved folders".into()))
    }
}

/// Read every selected session into one bundle and lower it to candidates.
/// Each store is opened once. A session that cannot be read becomes a
/// diagnostic-only candidate.
fn preview_sessions(
    approved: &[PathBuf],
    artifacts: &[&DiscoveredArtifact],
    discovery: &DiscoveryResult,
) -> Result<Vec<ImportCandidate>, BridgeError> {
    use crate::import_bundle::{self, BundleSource, TransferBundle};
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
    let mut stores: HashMap<PathBuf, Result<rusqlite::Connection, String>> = HashMap::new();
    let mut skipped: HashMap<String, Vec<ImportDiagnostic>> = HashMap::new();
    for artifact in artifacts {
        let read = parse_source_ref(&artifact.canonical_source_ref)
            .ok_or_else(|| BridgeError::Invalid("Not an OpenCode session reference".into()))
            .and_then(|(store, session)| {
                let store = approved_store(approved, &store)?;
                let conn = stores
                    .entry(store.clone())
                    .or_insert_with(|| store::open(&store).map_err(|error| error.to_string()))
                    .as_ref()
                    .map_err(|error| BridgeError::Invalid(format!("OpenCode's store could not be opened: {error}")))?;
                let row = store::find_session(conn, &session)
                    .map_err(|error| BridgeError::Invalid(format!("OpenCode's store could not be read: {error}")))?
                    .ok_or_else(|| BridgeError::Invalid("OpenCode no longer has this session".into()))?;
                let home = store.parent().map_or_else(|| store.clone(), Path::to_path_buf);
                store::read_session(
                    conn,
                    &row,
                    &store::SessionSource {
                        store: &store,
                        home: &home,
                        source_ref: artifact.canonical_source_ref.clone(),
                        label: artifact.source_label.clone(),
                    },
                )
                .map_err(|error| BridgeError::Invalid(format!("The session could not be read: {error}")))
            });
        let parsed = match read {
            Ok(parsed) => parsed,
            Err(error) => {
                out.push(unsupported_candidate(artifact, discovery, &error.to_string()));
                continue;
            }
        };
        if parsed.chat.entries.is_empty() {
            out.push(unsupported_candidate(artifact, discovery, "1 chat: no messages Bridge can show"));
            continue;
        }
        skipped.insert(parsed.chat.source_ref.clone(), skipped_diagnostics(&parsed));
        bundle.chats.push(parsed.chat);
    }
    let lowered = import_bundle::lower(&import_bundle::resolve(vec![bundle])?);
    for mut candidate in lowered.candidates {
        if let Some(diagnostics) = skipped.remove(&candidate.source.canonical_source_ref) {
            candidate.diagnostics.extend(diagnostics);
        }
        out.push(candidate);
    }
    Ok(out)
}

fn skipped_diagnostics(parsed: &store::ParsedSession) -> Vec<ImportDiagnostic> {
    parsed
        .skipped
        .iter()
        .map(|(reason, count)| ImportDiagnostic {
            code: "transcript_record_skipped".into(),
            severity: "info".into(),
            classification: SourceClassification::Documented,
            message: format!("Skipped {count} '{reason}' record(s) Bridge has no place for"),
            recovery: None,
            source_label: Some(parsed.chat.source_label.clone()),
        })
        .collect()
}

fn unsupported_diagnostic(code: &str, message: &str, source_label: impl Into<Option<String>>) -> ImportDiagnostic {
    ImportDiagnostic {
        code: code.into(),
        severity: "warning".into(),
        classification: SourceClassification::UnsupportedPrivate,
        message: message.into(),
        recovery: Some("Choose a supported OpenCode store".into()),
        source_label: source_label.into(),
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
        diagnostics: vec![unsupported_diagnostic("artifact_preview_failed", reason, Some(artifact.source_label.clone()))],
        normalized_payload: Value::Object(Map::new()),
    }
}

/// What the background import reads from OpenCode's stores: every session,
/// oldest first so a parent comes before its subagents, from the session
/// tables and message sizes only. Background runs are counted and left out.
pub fn job_work(roots: &[PathBuf]) -> Result<crate::claude_import::JobWork, BridgeError> {
    let mut artifacts = Vec::new();
    let mut left_out = 0u64;
    for store in stores_under(roots) {
        for row in read_store(&store)? {
            if is_background(&row.directory) {
                left_out += 1;
                continue;
            }
            artifacts.push(artifact_for(&store, &row));
        }
    }
    artifacts.sort_by_cached_key(job_key);
    Ok(crate::claude_import::JobWork { roots: roots.to_vec(), artifacts, left_out })
}

/// The order a job reads in, and its resume cursor: by store, then by when
/// the session started, then by id. A subagent starts after its parent, and
/// a session started later sorts after the cursor.
pub fn job_key(artifact: &DiscoveredArtifact) -> String {
    let (store, session) = parse_source_ref(&artifact.canonical_source_ref)
        .map(|(store, session)| (store.to_string_lossy().into_owned(), session))
        .unwrap_or_else(|| (artifact.canonical_source_ref.clone(), String::new()));
    format!("1:{store}#{}#{session}", artifact.started_at.as_deref().unwrap_or(""))
}

pub fn job_discovery(roots: &[PathBuf], artifacts: Vec<DiscoveredArtifact>) -> DiscoveryResult {
    discovery(roots, artifacts, None)
}

/// What one store holds, for the Import page: chats (subagents come over
/// under their parent and are not counted), their folders, and background
/// runs. Reads the session tables only.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StoreCounts {
    pub chats: u64,
    pub projects: u64,
    pub background: u64,
    /// The chats' folders, so counts across several stores add up projects
    /// once.
    pub folders: std::collections::BTreeSet<String>,
}

pub fn count_sessions(store: &Path) -> Option<StoreCounts> {
    let conn = store::open(store).ok()?;
    let rows = store::list_session_heads(&conn).ok()?;
    let mut counts = StoreCounts::default();
    for (directory, is_child) in rows {
        if is_background(&directory) {
            counts.background += 1;
        } else if !is_child {
            counts.chats += 1;
            counts.folders.insert(directory);
        }
    }
    counts.projects = counts.folders.len() as u64;
    Some(counts)
}

/// Whether OpenCode still keeps `session` in the store under a data folder.
pub fn still_has(data_folder: &Path, session: &str) -> bool {
    store_has(&data_folder.join(STORE_FILE), session)
}

/// Whether a store file still holds `session`.
pub fn store_has(store: &Path, session: &str) -> bool {
    store.is_file()
        && store::open(store)
            .ok()
            .and_then(|conn| store::find_session(&conn, session).ok().flatten())
            .is_some()
}

#[cfg(test)]
#[path = "opencode_import/tests.rs"]
mod tests;

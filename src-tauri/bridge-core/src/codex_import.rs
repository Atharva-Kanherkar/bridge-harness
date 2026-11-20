//! Codex chats into Bridge (#1054): the rollouts under a Codex home
//! (`CODEX_HOME`, else `~/.codex`), read by [`rollout`] into the transfer
//! bundle and written by the same pipeline as Claude Code's.
//!
//! Only rollouts are read. `auth.json`, `logs_*.sqlite`, `mcp-oauth-locks/`
//! and the other private stores never are. Memories stay with Codex, which
//! reads its own.

pub mod rollout;

use crate::external_import::{
    canonicalize_approved_path, content_hash, deterministic_identity, source_now, validate_schema_version,
    CandidateKind, DiscoveredArtifact, DiscoveryRequest, DiscoveryResult, DiscoverySelection,
    ExternalHarnessImporter, FixtureFormat, FixtureManifest, ImportCandidate, ImportDiagnostic,
    NormalizedImportCandidate, RedactionSummary, SchemaGate, SourceClassification, Stability, ValidationResult,
};
use crate::BridgeError;
use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// The bundle tool id, as detection and the Import page name it.
pub const PROVIDER: &str = "codex";
pub const ADAPTER_VERSION: &str = "1";
pub const ROLLOUT_GATE: &str = "codex-rollout-v1";
const MAX_DISCOVERY_DEPTH: usize = 6;

#[derive(Debug, Default, Clone, Copy)]
pub struct CodexImporter;

fn supported_format_versions() -> std::collections::BTreeMap<String, String> {
    std::collections::BTreeMap::from([(ROLLOUT_GATE.to_owned(), ROLLOUT_GATE.to_owned())])
}

/// The Codex home this daemon's Codex uses: `CODEX_HOME`, else `~/.codex`.
pub fn machine_codex_homes() -> Vec<PathBuf> {
    let home = std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")));
    existing_folders(home.into_iter(), Path::is_dir)
}

/// Each folder that exists and passes `keep`, by its real path, once.
pub(crate) fn existing_folders(candidates: impl Iterator<Item = PathBuf>, keep: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for candidate in candidates {
        if let Ok(real) = candidate.canonicalize() {
            if keep(&real) && !out.contains(&real) {
                out.push(real);
            }
        }
    }
    out
}

/// A Codex home: one this machine uses, or a folder laid out like one.
fn is_codex_home(root: &Path, known: &[PathBuf]) -> bool {
    known.iter().any(|home| home == root) || root.join("sessions").is_dir() || root.join("archived_sessions").is_dir()
}

fn is_rollout(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("jsonl")
        && path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with("rollout-"))
}

fn is_archived(path: &Path) -> bool {
    path.components().any(|part| part.as_os_str() == "archived_sessions")
}

/// The home a rollout belongs to: the folder above `sessions` or
/// `archived_sessions`.
fn home_of(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|ancestor| matches!(ancestor.file_name().and_then(|name| name.to_str()), Some("sessions" | "archived_sessions")))
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

fn collect_rollouts(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_DISCOVERY_DEPTH {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            collect_rollouts(&path, depth + 1, out);
        } else if kind.is_file() && is_rollout(&path) {
            out.push(path);
        }
    }
}

fn push_artifact(path: &Path, approved: &[PathBuf], artifacts: &mut Vec<DiscoveredArtifact>) -> Result<(), BridgeError> {
    let canonical = canonicalize_approved_path(approved, path)?;
    let metadata = fs::metadata(&canonical)?;
    let label = label_for(&canonical);
    artifacts.push(DiscoveredArtifact {
        artifact_id: deterministic_identity(
            PROVIDER,
            &canonical.to_string_lossy(),
            Some(&label),
            &metadata.len().to_string(),
            &CandidateKind::Conversation,
        ),
        canonical_source_ref: canonical.to_string_lossy().into_owned(),
        source_label: label,
        kind: CandidateKind::Conversation,
        classification: SourceClassification::VersionGatedPrivate,
        stability: Stability::VersionGated,
        estimated_bytes: metadata.len(),
        modified_at: metadata.modified().ok().map(|time| DateTime::<Utc>::from(time).to_rfc3339()),
        required_schema_gate: Some(ROLLOUT_GATE.into()),
        title: None,
        project_path: None,
        started_at: None,
    });
    Ok(())
}

/// `sessions/2026/11/17/rollout-….jsonl`, never a full path.
fn label_for(path: &Path) -> String {
    let parts: Vec<String> = path
        .components()
        .rev()
        .take(5)
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    let start = parts.iter().position(|part| part == "sessions" || part == "archived_sessions");
    let keep = start.map_or(1, |index| index + 1);
    parts.into_iter().take(keep).rev().collect::<Vec<_>>().join("/")
}

/// Every rollout under the approved Codex homes, sorted.
fn rollouts_under(roots: &[PathBuf], known: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for root in roots.iter().filter(|root| is_codex_home(root, known)) {
        collect_rollouts(&root.join("sessions"), 0, &mut paths);
        collect_rollouts(&root.join("archived_sessions"), 0, &mut paths);
    }
    paths.sort();
    paths.dedup();
    paths
}

impl CodexImporter {
    pub fn discover_with_homes(&self, request: &DiscoveryRequest, known: &[PathBuf]) -> Result<DiscoveryResult, BridgeError> {
        if request.provider != PROVIDER {
            return Err(BridgeError::Invalid(format!("Codex importer cannot discover provider '{}'", request.provider)));
        }
        let roots = canonical_roots(&request.approved_roots)?;
        if roots.is_empty() {
            return Err(BridgeError::Invalid("Choose a Codex folder before discovery".into()));
        }
        let mut artifacts = Vec::new();
        for path in rollouts_under(&roots, known) {
            push_artifact(&path, &roots, &mut artifacts)?;
        }
        Ok(discovery(&roots, artifacts, request.source_version.clone()))
    }
}

fn canonical_roots(roots: &[String]) -> Result<Vec<PathBuf>, BridgeError> {
    let mut out = Vec::new();
    for root in roots {
        let path = Path::new(root)
            .canonicalize()
            .map_err(|error| BridgeError::Invalid(format!("Approved Codex folder is unavailable: {error}")))?;
        if !path.is_dir() {
            return Err(BridgeError::Invalid("Approved Codex folders must be directories".into()));
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

impl ExternalHarnessImporter for CodexImporter {
    fn discover(&self, request: &DiscoveryRequest) -> Result<DiscoveryResult, BridgeError> {
        self.discover_with_homes(request, &machine_codex_homes())
    }

    fn preview(&self, discovery: &DiscoveryResult, selection: &DiscoverySelection) -> Result<Vec<ImportCandidate>, BridgeError> {
        if discovery.provider != PROVIDER {
            return Err(BridgeError::Invalid("Discovery result belongs to another provider".into()));
        }
        let gate = validate_schema_version(
            &SchemaGate { format: ROLLOUT_GATE.into(), allowed_versions: vec![ROLLOUT_GATE.into()] },
            discovery.format_versions.get(ROLLOUT_GATE).map(String::as_str),
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
        preview_rollouts(&approved, &artifacts, discovery)
    }

    fn validate(&self, candidate: &ImportCandidate, schema_gate: &SchemaGate) -> ValidationResult {
        if candidate.source.provider != PROVIDER {
            return ValidationResult {
                accepted: false,
                diagnostics: vec![unsupported_diagnostic("provider_mismatch", "Candidate does not belong to Codex", None)],
            };
        }
        if matches!(candidate.stability, Stability::VersionGated) {
            return validate_schema_version(schema_gate, candidate.source.schema_version.as_deref());
        }
        ValidationResult { accepted: candidate.redaction_summary.safely_representable, diagnostics: candidate.diagnostics.clone() }
    }

    fn normalize(&self, candidate: ImportCandidate) -> Result<NormalizedImportCandidate, BridgeError> {
        if candidate.source.provider != PROVIDER || !candidate.redaction_summary.safely_representable {
            return Err(BridgeError::Invalid("Codex candidate is not safe to normalize".into()));
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
                source: "Codex rollout JSONL (sessions/, archived_sessions/)".into(),
                classification: SourceClassification::VersionGatedPrivate,
                versions: vec![ROLLOUT_GATE.into()],
            }],
        }
    }
}

/// Codex threads that are themselves imports of another agent's session
/// (`external_agent_session_imports.json`): thread id to the source file.
fn external_imports(home: &Path) -> HashMap<String, String> {
    let Ok(text) = fs::read_to_string(home.join("external_agent_session_imports.json")) else {
        return HashMap::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return HashMap::new();
    };
    value
        .get("records")
        .and_then(Value::as_array)
        .map(|records| {
            records
                .iter()
                .filter_map(|record| {
                    Some((
                        record.get("imported_thread_id")?.as_str()?.to_owned(),
                        record.get("source_path")?.as_str()?.to_owned(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Read every selected rollout into one bundle and lower it to candidates.
/// A rollout that cannot be read becomes a diagnostic-only candidate.
fn preview_rollouts(
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
        schema_version: Some(ROLLOUT_GATE.into()),
        adapter_version: ADAPTER_VERSION.into(),
        roots: discovery.approved_roots.clone(),
        detected_at: discovery.discovered_at.clone(),
    });
    let mut titles: HashMap<PathBuf, HashMap<String, String>> = HashMap::new();
    let mut imports: HashMap<PathBuf, HashMap<String, String>> = HashMap::new();
    let mut skipped: HashMap<String, Vec<ImportDiagnostic>> = HashMap::new();
    let mut copies = 0u32;
    for artifact in artifacts {
        let read = canonicalize_approved_path(approved, Path::new(&artifact.canonical_source_ref)).and_then(|path| {
            let home = home_of(&path).unwrap_or_else(|| path.clone());
            let titles = titles.entry(home.clone()).or_insert_with(|| rollout::read_titles(&home));
            let parsed = rollout::read_rollout(&rollout::RolloutSource {
                path: &path,
                home: &home,
                titles,
                archived: is_archived(&path),
                label: artifact.source_label.clone(),
            })
            .map_err(|error| BridgeError::Invalid(format!("Rollout could not be read: {error}")))?;
            Ok((home, parsed))
        });
        let (home, parsed) = match read {
            Ok(read) => read,
            Err(error) => {
                out.push(unsupported_candidate(artifact, discovery, &error.to_string()));
                continue;
            }
        };
        // A copy of another agent's session that nobody typed into here: the
        // conversation comes over from its own tool.
        let copied_from = parsed
            .meta
            .thread_id
            .as_ref()
            .and_then(|id| imports.entry(home.clone()).or_insert_with(|| external_imports(&home)).get(id).cloned());
        if parsed.typed_messages == 0 && copied_from.is_some_and(|source| Path::new(&source).is_file()) {
            copies += 1;
            continue;
        }
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
    if copies > 0 {
        if let Some(first) = out.iter_mut().find(|candidate| candidate.kind != CandidateKind::Unsupported) {
            first.diagnostics.push(ImportDiagnostic {
                code: "excluded_copies".into(),
                severity: "info".into(),
                classification: SourceClassification::Documented,
                message: format!("{copies} Codex copies of another tool's chat, which come over from that tool"),
                recovery: None,
                source_label: None,
            });
        }
    }
    Ok(out)
}

fn skipped_diagnostics(parsed: &rollout::ParsedRollout) -> Vec<ImportDiagnostic> {
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
        recovery: Some("Choose a supported Codex rollout".into()),
        source_label: source_label.into(),
    }
}

fn unsupported_candidate(artifact: &DiscoveredArtifact, discovery: &DiscoveryResult, reason: &str) -> ImportCandidate {
    let source = source_now(
        PROVIDER,
        ADAPTER_VERSION,
        Path::new(&artifact.canonical_source_ref),
        discovery.source_version.clone(),
        discovery.format_versions.get(ROLLOUT_GATE).cloned(),
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
        title: artifact.source_label.clone(),
        created_at: artifact.modified_at.clone(),
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

/// What the background import reads from Codex homes: every rollout, in a
/// fixed order, from listings and first lines only. Threads that ran in the
/// OS temp folder are background runs: counted, and left out.
pub fn job_work(roots: &[PathBuf]) -> Result<crate::claude_import::JobWork, BridgeError> {
    let mut artifacts = Vec::new();
    let mut left_out = 0u64;
    for path in rollouts_under(roots, roots) {
        let background = rollout::read_meta(&path)
            .and_then(|meta| meta.cwd)
            .is_some_and(|cwd| is_temp_folder(&cwd));
        if background {
            left_out += 1;
            continue;
        }
        push_artifact(&path, roots, &mut artifacts)?;
    }
    artifacts.sort_by_cached_key(job_key);
    Ok(crate::claude_import::JobWork { roots: roots.to_vec(), artifacts, left_out })
}

/// The OS temp folder, where Bridge runs probes and background work.
pub fn is_temp_folder(cwd: &str) -> bool {
    ["/private/var/folders/", "/var/folders/", "/private/tmp/", "/tmp/"]
        .iter()
        .any(|prefix| format!("{cwd}/").starts_with(prefix))
        || std::env::temp_dir().to_str().is_some_and(|temp| cwd.starts_with(temp.trim_end_matches('/')))
}

/// The order a job reads in, and its resume cursor: by path, which is by date.
pub fn job_key(artifact: &DiscoveredArtifact) -> String {
    format!("1:{}", artifact.canonical_source_ref)
}

pub fn job_discovery(roots: &[PathBuf], artifacts: Vec<DiscoveredArtifact>) -> DiscoveryResult {
    discovery(roots, artifacts, None)
}

/// The folder a rollout's thread ran in, from its first record.
pub fn transcript_folder(path: &Path) -> Option<String> {
    rollout::read_meta(path).and_then(|meta| meta.cwd)
}

/// Every rollout under one Codex home, sorted.
pub(crate) fn list_rollouts(home: &Path) -> Vec<PathBuf> {
    rollouts_under(&[home.to_path_buf()], &[home.to_path_buf()])
}

/// Where Codex keeps a thread's rollout under a home, if it still does.
pub fn find_rollout(home: &Path, thread_id: &str) -> Option<PathBuf> {
    let mut paths = Vec::new();
    collect_rollouts(&home.join("sessions"), 0, &mut paths);
    collect_rollouts(&home.join("archived_sessions"), 0, &mut paths);
    paths.into_iter().find(|path| {
        path.file_stem().and_then(|stem| stem.to_str()).is_some_and(|stem| stem.ends_with(thread_id))
    })
}

#[cfg(test)]
#[path = "codex_import/tests.rs"]
mod tests;

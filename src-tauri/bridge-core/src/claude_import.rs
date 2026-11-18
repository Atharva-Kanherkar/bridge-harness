//! Claude Code source adapter for the shared external-import pipeline.
//!
//! Discovery is deliberately filesystem-only and requires explicit roots.
//! Documented Markdown/configuration surfaces are stable; auto memory and
//! transcripts are parsed only when the caller supplies the named schema gate.

use crate::external_import::{
    canonicalize_approved_path, content_hash, deterministic_identity, sanitize_import_payload,
    source_now, validate_schema_version, CandidateKind, DiscoveredArtifact, DiscoveryRequest,
    DiscoveryResult, DiscoverySelection, ExternalHarnessImporter, FixtureFormat, FixtureManifest,
    ImportCandidate, ImportDiagnostic, NormalizedImportCandidate, RedactionSummary, SchemaGate,
    SourceClassification, Stability, ValidationResult,
};
use crate::BridgeError;
use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub mod transcript;

pub const PROVIDER: &str = "claude_code";
pub const ADAPTER_VERSION: &str = "2";
pub const AUTO_MEMORY_GATE: &str = "claude-auto-memory-v1";
pub const TRANSCRIPT_GATE: &str = "claude-jsonl-v2";
/// The gate the v1 reader wrote into its payloads; kept so v2 can recompute
/// a v1 import's identity exactly.
const LEGACY_TRANSCRIPT_GATE: &str = "claude-jsonl-v1";
pub const SETTINGS_GATE: &str = "claude-settings-v1";
const MAX_SOURCE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_DISCOVERY_DEPTH: usize = 20;

#[derive(Debug, Default, Clone, Copy)]
pub struct ClaudeCodeImporter;

/// The only format versions this adapter build can actually parse, keyed by
/// gate id. `preview()`'s schema-version gate checks a candidate's declared
/// version against this map — never against anything a wire caller supplied.
fn supported_format_versions() -> std::collections::BTreeMap<String, String> {
    std::collections::BTreeMap::from([
        (AUTO_MEMORY_GATE.to_string(), AUTO_MEMORY_GATE.to_string()),
        (TRANSCRIPT_GATE.to_string(), TRANSCRIPT_GATE.to_string()),
        (SETTINGS_GATE.to_string(), SETTINGS_GATE.to_string()),
    ])
}

impl ClaudeCodeImporter {
    /// Discovery that knows this machine's config homes up front. A folder
    /// detection found is a config home whatever its layout, even before
    /// Claude Code has saved a project in it.
    pub fn discover_with_homes(
        &self,
        request: &DiscoveryRequest,
        known_homes: &[PathBuf],
    ) -> Result<DiscoveryResult, BridgeError> {
        if request.provider != PROVIDER {
            return Err(BridgeError::Invalid(format!(
                "Claude Code importer cannot discover provider '{}'",
                request.provider
            )));
        }
        if request.approved_roots.is_empty() && request.selected_export.is_none() {
            return Err(BridgeError::Invalid(
                "Choose at least one Claude Code root or export before discovery".into(),
            ));
        }

        let discovery_roots = canonical_roots(&request.approved_roots)?;
        let mut approved_roots = discovery_roots.clone();
        let selected_export = request
            .selected_export
            .as_deref()
            .map(Path::new)
            .map(Path::canonicalize)
            .transpose()
            .map_err(|error| {
                BridgeError::Invalid(format!("Selected Claude export is unavailable: {error}"))
            })?;
        if let Some(parent) = selected_export.as_ref().and_then(|path| path.parent()) {
            approved_roots.push(parent.to_path_buf());
            approved_roots.sort();
            approved_roots.dedup();
        }
        let mut artifacts = Vec::new();
        let mut diagnostics = Vec::new();
        for root in &discovery_roots {
            let is_home = is_claude_home(root, known_homes);
            discover_root(root, is_home, &approved_roots, &mut artifacts, &mut diagnostics)?;
        }
        if let Some(export) = selected_export {
            let path = canonicalize_approved_path(&approved_roots, &export)?;
            if path.extension().and_then(|part| part.to_str()) == Some("jsonl") {
                push_artifact(
                    &path,
                    &approved_roots,
                    CandidateKind::Conversation,
                    SourceClassification::VersionGatedPrivate,
                    Stability::VersionGated,
                    Some(TRANSCRIPT_GATE),
                    &mut artifacts,
                )?;
            } else {
                diagnostics.push(unsupported_diagnostic(
                    "unsupported_export",
                    "Selected Claude export is not an allowlisted JSONL fixture format",
                    safe_label(&path, &approved_roots),
                ));
            }
        }
        artifacts.sort_by(|left, right| left.source_label.cmp(&right.source_label));
        artifacts.dedup_by(|left, right| left.canonical_source_ref == right.canonical_source_ref);
        // The review list shows each chat by title and folder; read only the
        // first and last few KiB of each transcript for that.
        for artifact in artifacts
            .iter_mut()
            .filter(|artifact| artifact.kind == CandidateKind::Conversation)
        {
            let peek = transcript::peek_session(Path::new(&artifact.canonical_source_ref));
            let (title, _) = sanitize_import_payload(&Value::from(peek.title));
            let (cwd, _) = sanitize_import_payload(&Value::from(peek.cwd));
            artifact.title = title.as_str().map(str::to_owned);
            artifact.project_path = cwd.as_str().map(str::to_owned);
            artifact.started_at = peek.started_at;
        }
        Ok(DiscoveryResult {
            // Opaque server-side cache key: the caller can no longer hand-build
            // a `DiscoveryResult` at `preview`/`commit` time and have it
            // accepted, because those calls look this id up in the daemon's own
            // discovery cache instead of trusting a client-supplied blob.
            discovery_id: format!("disc_{}", uuid::Uuid::new_v4().simple()),
            provider: PROVIDER.into(),
            approved_roots: approved_roots
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect(),
            source_version: request.source_version.clone(),
            // The set of format versions *this adapter build* can parse, not
            // whatever the caller claims — the wire request's `format_versions`
            // is intentionally not read here. Echoing it back would make the
            // schema-version gate in `preview()` compare the caller's claim
            // against itself, which can never fail closed.
            format_versions: supported_format_versions(),
            artifacts,
            diagnostics,
            discovered_at: Utc::now().to_rfc3339(),
        })
    }
}

impl ExternalHarnessImporter for ClaudeCodeImporter {
    fn discover(&self, request: &DiscoveryRequest) -> Result<DiscoveryResult, BridgeError> {
        self.discover_with_homes(request, &machine_config_roots())
    }

    fn preview(
        &self,
        discovery: &DiscoveryResult,
        selection: &DiscoverySelection,
    ) -> Result<Vec<ImportCandidate>, BridgeError> {
        if discovery.provider != PROVIDER {
            return Err(BridgeError::Invalid(
                "Discovery result belongs to another provider".into(),
            ));
        }
        let approved_roots: Vec<PathBuf> =
            discovery.approved_roots.iter().map(PathBuf::from).collect();
        let selected: HashSet<&str> = selection.artifact_ids.iter().map(String::as_str).collect();
        let mut candidates = Vec::new();
        let (conversations, others): (Vec<&DiscoveredArtifact>, Vec<&DiscoveredArtifact>) = discovery
            .artifacts
            .iter()
            .filter(|artifact| selected.contains(artifact.artifact_id.as_str()))
            .partition(|artifact| artifact.kind == CandidateKind::Conversation);
        candidates.extend(preview_conversations(&approved_roots, &conversations, discovery)?);
        for artifact in others {
            // Each artifact previews independently: one unreadable file, one
            // unsupported record inside a transcript that turned out to carry
            // nothing else, or one schema-gate rejection must not blank out
            // every other artifact the caller selected in the same batch.
            match preview_one_artifact(&approved_roots, artifact, discovery) {
                Ok(items) => candidates.extend(items),
                Err(error) => {
                    candidates.push(unsupported_candidate(artifact, discovery, &error.to_string()))
                }
            }
        }
        candidates.sort_by(|left, right| {
            left.kind
                .as_str()
                .cmp(right.kind.as_str())
                .then_with(|| left.title.cmp(&right.title))
        });
        Ok(candidates)
    }

    fn validate(&self, candidate: &ImportCandidate, schema_gate: &SchemaGate) -> ValidationResult {
        if candidate.source.provider != PROVIDER {
            return ValidationResult {
                accepted: false,
                diagnostics: vec![unsupported_diagnostic(
                    "provider_mismatch",
                    "Candidate does not belong to Claude Code",
                    None,
                )],
            };
        }
        if matches!(candidate.stability, Stability::VersionGated) {
            return validate_schema_version(
                schema_gate,
                candidate.source.schema_version.as_deref(),
            );
        }
        ValidationResult {
            accepted: candidate.redaction_summary.safely_representable,
            diagnostics: candidate.diagnostics.clone(),
        }
    }

    fn normalize(
        &self,
        candidate: ImportCandidate,
    ) -> Result<NormalizedImportCandidate, BridgeError> {
        if candidate.source.provider != PROVIDER
            || !candidate.redaction_summary.safely_representable
        {
            return Err(BridgeError::Invalid(
                "Claude candidate is not safe to normalize".into(),
            ));
        }
        let deterministic_bridge_id = deterministic_identity(
            PROVIDER,
            &candidate.source.canonical_source_ref,
            candidate.source_native_id.as_deref(),
            &candidate.content_hash,
            &candidate.kind,
        );
        Ok(NormalizedImportCandidate {
            candidate,
            deterministic_bridge_id,
        })
    }

    fn fixture_manifest(&self) -> FixtureManifest {
        FixtureManifest {
            provider: PROVIDER.into(),
            adapter_version: ADAPTER_VERSION.into(),
            formats: vec![
                FixtureFormat {
                    source: "CLAUDE.md and .claude/rules/**/*.md".into(),
                    classification: SourceClassification::Documented,
                    versions: vec!["markdown".into()],
                },
                FixtureFormat {
                    source: "Claude auto-memory Markdown".into(),
                    classification: SourceClassification::VersionGatedPrivate,
                    versions: vec![AUTO_MEMORY_GATE.into()],
                },
                FixtureFormat {
                    source: "Claude session JSONL".into(),
                    classification: SourceClassification::VersionGatedPrivate,
                    versions: vec![TRANSCRIPT_GATE.into()],
                },
                FixtureFormat {
                    source: "Claude settings/setup configuration".into(),
                    classification: SourceClassification::Documented,
                    versions: vec![SETTINGS_GATE.into()],
                },
            ],
        }
    }
}

fn preview_one_artifact(
    approved_roots: &[PathBuf],
    artifact: &DiscoveredArtifact,
    discovery: &DiscoveryResult,
) -> Result<Vec<ImportCandidate>, BridgeError> {
    let source_path =
        canonicalize_approved_path(approved_roots, Path::new(&artifact.canonical_source_ref))?;
    if artifact.kind != CandidateKind::Conversation {
        enforce_size(&source_path)?;
    }
    if let Some(gate) = artifact.required_schema_gate.as_deref() {
        let actual = discovery.format_versions.get(gate).map(String::as_str);
        let validation = validate_schema_version(
            &SchemaGate {
                format: gate.into(),
                allowed_versions: vec![gate.into()],
            },
            actual,
        );
        if !validation.accepted {
            return Err(BridgeError::Invalid(
                validation.diagnostics[0].message.clone(),
            ));
        }
    }
    Ok(match artifact.kind {
        CandidateKind::Conversation => {
            vec![preview_transcript(&source_path, artifact, discovery)?]
        }
        CandidateKind::Memory => vec![preview_markdown(
            &source_path,
            artifact,
            discovery,
            CandidateKind::Memory,
        )?],
        CandidateKind::Instruction
        | CandidateKind::Rule
        | CandidateKind::Command
        | CandidateKind::Skill
        | CandidateKind::Agent => vec![preview_markdown(
            &source_path,
            artifact,
            discovery,
            artifact.kind.clone(),
        )?],
        CandidateKind::Prompt => preview_settings(&source_path, artifact, discovery)?,
        CandidateKind::McpServer => preview_mcp(&source_path, artifact, discovery)?,
        ref other => {
            return Err(BridgeError::Invalid(format!(
                "Claude {} artifacts are not an import candidate on their own",
                other.as_str()
            )))
        }
    })
}

/// A diagnostic-only stand-in for an artifact whose preview failed — an
/// unreadable file, a rejected schema gate, or (most commonly) a transcript
/// that, after skipping every record type Bridge does not carry into
/// history, had nothing importable left. `validate_candidate_integrity`
/// refuses to commit a `CandidateKind::Unsupported` candidate, so this can
/// only ever surface as information in the preview list.
fn unsupported_candidate(
    artifact: &DiscoveredArtifact,
    discovery: &DiscoveryResult,
    reason: &str,
) -> ImportCandidate {
    let source = source_now(
        PROVIDER,
        ADAPTER_VERSION,
        Path::new(&artifact.canonical_source_ref),
        discovery.source_version.clone(),
        artifact
            .required_schema_gate
            .as_ref()
            .and_then(|gate| discovery.format_versions.get(gate))
            .cloned(),
    );
    let candidate_id = deterministic_identity(
        PROVIDER,
        &source.canonical_source_ref,
        Some("unsupported"),
        &content_hash(&Value::String(reason.to_owned())),
        &CandidateKind::Unsupported,
    );
    ImportCandidate {
        candidate_id,
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
        redaction_summary: RedactionSummary {
            safely_representable: false,
            ..RedactionSummary::default()
        },
        diagnostics: vec![unsupported_diagnostic(
            "artifact_preview_failed",
            reason,
            Some(artifact.source_label.clone()),
        )],
        normalized_payload: Value::Object(Map::new()),
    }
}

fn canonical_roots(roots: &[String]) -> Result<Vec<PathBuf>, BridgeError> {
    let mut canonical = Vec::new();
    for root in roots {
        let path = Path::new(root).canonicalize().map_err(|error| {
            BridgeError::Invalid(format!("Approved Claude Code root is unavailable: {error}"))
        })?;
        if !path.is_dir() {
            return Err(BridgeError::Invalid(
                "Approved Claude Code roots must be directories".into(),
            ));
        }
        canonical.push(path);
    }
    canonical.sort();
    canonical.dedup();
    Ok(canonical)
}

/// A Claude config home, whatever it is called (`~/.claude`, or the folder
/// `CLAUDE_CONFIG_DIR` or an account names). A folder this machine uses as a
/// config home is one by identity, even before Claude Code has saved a
/// project in it. Any other folder is recognised by its layout: a
/// `projects/` folder of Claude's project folders, which are named after a
/// path with `/` written as `-`, so they start with `-`.
fn is_claude_home(root: &Path, known_homes: &[PathBuf]) -> bool {
    if known_homes.iter().any(|home| home == root)
        || root.file_name().and_then(|name| name.to_str()) == Some(".claude")
    {
        return true;
    }
    let Ok(projects) = fs::read_dir(root.join("projects")) else {
        return false;
    };
    projects.flatten().any(|entry| {
        entry.path().is_dir()
            && entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with('-'))
    })
}

fn discover_root(
    root: &Path,
    is_claude_home: bool,
    approved_roots: &[PathBuf],
    artifacts: &mut Vec<DiscoveredArtifact>,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> Result<(), BridgeError> {
    let candidates = if is_claude_home {
        vec![
            (
                root.join("CLAUDE.md"),
                CandidateKind::Instruction,
                Stability::Stable,
                None,
            ),
            (
                root.join("settings.json"),
                CandidateKind::Prompt,
                Stability::Stable,
                Some(SETTINGS_GATE),
            ),
        ]
    } else {
        vec![
            (
                root.join("CLAUDE.md"),
                CandidateKind::Instruction,
                Stability::Stable,
                None,
            ),
            (
                root.join("CLAUDE.local.md"),
                CandidateKind::Instruction,
                Stability::Stable,
                None,
            ),
            (
                root.join(".claude/CLAUDE.md"),
                CandidateKind::Instruction,
                Stability::Stable,
                None,
            ),
            (
                root.join(".claude/settings.json"),
                CandidateKind::Prompt,
                Stability::Stable,
                Some(SETTINGS_GATE),
            ),
            (
                root.join(".claude/settings.local.json"),
                CandidateKind::Prompt,
                Stability::Stable,
                Some(SETTINGS_GATE),
            ),
            (
                root.join(".mcp.json"),
                CandidateKind::McpServer,
                Stability::Stable,
                Some(SETTINGS_GATE),
            ),
        ]
    };
    for (path, kind, stability, gate) in candidates {
        if path.is_file() {
            push_artifact(
                &path,
                approved_roots,
                kind,
                SourceClassification::Documented,
                stability,
                gate,
                artifacts,
            )?;
        }
    }

    let claude_dir = if is_claude_home {
        root.to_path_buf()
    } else {
        root.join(".claude")
    };
    for (relative, kind) in [
        ("rules", CandidateKind::Rule),
        ("commands", CandidateKind::Command),
        ("agents", CandidateKind::Agent),
        ("skills", CandidateKind::Skill),
    ] {
        collect_markdown(
            &claude_dir.join(relative),
            approved_roots,
            kind,
            SourceClassification::Documented,
            Stability::Stable,
            None,
            artifacts,
            diagnostics,
        )?;
    }
    if is_claude_home {
        collect_named_files(
            &root.join("projects"),
            approved_roots,
            |path| {
                path.parent()
                    .and_then(Path::file_name)
                    .and_then(|name| name.to_str())
                    == Some("memory")
                    && path.extension().and_then(|part| part.to_str()) == Some("md")
            },
            CandidateKind::Memory,
            SourceClassification::VersionGatedPrivate,
            Stability::VersionGated,
            Some(AUTO_MEMORY_GATE),
            artifacts,
            diagnostics,
        )?;
        collect_named_files(
            &root.join("projects"),
            approved_roots,
            is_session_transcript,
            CandidateKind::Conversation,
            SourceClassification::VersionGatedPrivate,
            Stability::VersionGated,
            Some(TRANSCRIPT_GATE),
            artifacts,
            diagnostics,
        )?;
    }
    Ok(())
}

/// A session's own transcript is `projects/<slug>/<session>.jsonl`. Files
/// under `<session>/subagents/` belong to that session and are read with it.
fn is_session_transcript(path: &Path) -> bool {
    path.extension().and_then(|part| part.to_str()) == Some("jsonl")
        && !path
            .components()
            .any(|component| component.as_os_str() == "subagents")
        && path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .is_some_and(|name| name == "projects")
}

/// The identity the v1 reader recorded for this transcript: its first
/// message id, and its content hash recomputed by the retained v1 reader.
/// v1 refused files over 10 MB, so a file that has grown past that since it
/// was imported keeps its key but has no comparable hash: it has changed.
fn legacy_identity(
    approved_roots: &[PathBuf],
    artifact: &DiscoveredArtifact,
    discovery: &DiscoveryResult,
) -> Option<(String, Option<String>)> {
    let path = canonicalize_approved_path(approved_roots, Path::new(&artifact.canonical_source_ref)).ok()?;
    if fs::metadata(&path).ok()?.len() > MAX_SOURCE_BYTES {
        return v1_first_message_id(&path).map(|id| (id, None));
    }
    let v1 = preview_transcript(&path, artifact, discovery).ok()?;
    Some((v1.source_native_id?, Some(v1.content_hash)))
}

/// The key v1 gave a transcript: the id of the first record it accepted as a
/// message, found by streaming, with v1's own record rules.
fn v1_first_message_id(path: &Path) -> Option<String> {
    use std::io::BufRead;
    let file = fs::File::open(path).ok()?;
    let mut skipped = BTreeMap::new();
    for line in std::io::BufReader::new(file).lines() {
        let line = line.ok()?;
        let Ok(record) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        let Some(object) = record.as_object() else {
            continue;
        };
        let Some(record_type) = object.get("type").and_then(Value::as_str) else {
            continue;
        };
        if !matches!(record_type, "user" | "assistant") {
            continue;
        }
        if let Some((source_id, _)) = parse_transcript_record(object, record_type, 1, &mut skipped) {
            return Some(source_id);
        }
    }
    None
}

/// Every selected transcript, read at full fidelity into one transfer bundle,
/// merged and lowered to candidates. A transcript that cannot be read becomes
/// a diagnostic-only candidate; the others still import.
/// Read one selected transcript and its subagents, after the path and the
/// schema gate check out.
/// One chat among the selected transcripts: its file and its native session
/// (or bundle key). A file can hold several chats: inline subagents.
type ChatKey = (String, String);

fn chat_key(chat: &crate::import_bundle::BundleChat) -> ChatKey {
    let id = chat
        .native_ref
        .as_ref()
        .map(|native| native.session_id.clone())
        .unwrap_or_else(|| chat.key.clone());
    (chat.source_ref.clone(), id)
}

/// The same key for a lowered candidate, whose native id is the session id or key.
fn candidate_key(candidate: &ImportCandidate) -> ChatKey {
    (
        candidate.source.canonical_source_ref.clone(),
        candidate.source_native_id.clone().unwrap_or_default(),
    )
}

fn read_artifact_session(
    approved_roots: &[PathBuf],
    artifact: &DiscoveredArtifact,
    discovery: &DiscoveryResult,
) -> Result<transcript::SessionRead, BridgeError> {
    let path = canonicalize_approved_path(approved_roots, Path::new(&artifact.canonical_source_ref))?;
    let gate = discovery.format_versions.get(TRANSCRIPT_GATE).map(String::as_str);
    let validation = validate_schema_version(
        &SchemaGate { format: TRANSCRIPT_GATE.into(), allowed_versions: vec![TRANSCRIPT_GATE.into()] },
        gate,
    );
    if !validation.accepted {
        return Err(BridgeError::Invalid(validation.diagnostics[0].message.clone()));
    }
    transcript::read_session(&path, &artifact.source_label, approved_roots)
        .map_err(|error| BridgeError::Invalid(format!("Claude transcript is unreadable: {error}")))
}

fn subagent_failure_diagnostic(failure: &transcript::ChildFailure) -> ImportDiagnostic {
    ImportDiagnostic {
        code: "excluded_subagent".into(),
        severity: "warning".into(),
        classification: SourceClassification::Documented,
        message: format!("1 subagent: {} ({})", failure.reason, failure.label),
        recovery: None,
        source_label: Some(failure.label.clone()),
    }
}

/// One diagnostic per kind of record the reader skipped, with its count.
fn skipped_diagnostics(item: &transcript::ParsedTranscript) -> Vec<ImportDiagnostic> {
    item.skipped
        .0
        .iter()
        .map(|(reason, count)| ImportDiagnostic {
            code: "transcript_record_skipped".into(),
            severity: "info".into(),
            classification: SourceClassification::Documented,
            message: format!("Skipped {count} '{reason}' record(s) Bridge has no place for"),
            recovery: None,
            source_label: Some(item.chat.source_label.clone()),
        })
        .collect()
}

/// Put each thing lowering left out on the chat it came from, or on a
/// diagnostic-only stand-in when the whole chat was left out.
fn attach_exclusions(
    out: &mut Vec<ImportCandidate>,
    excluded: Vec<crate::import_bundle::BundleExclusion>,
    ref_by_label: &std::collections::HashMap<String, ChatKey>,
    artifacts: &[&DiscoveredArtifact],
    discovery: &DiscoveryResult,
) {
    for excluded in excluded {
        let diagnostic = ImportDiagnostic {
            code: format!("excluded_{}", excluded.category),
            severity: "warning".into(),
            classification: SourceClassification::Documented,
            message: format!("{} {}: {}", excluded.count, excluded.category.replace('_', " "), excluded.reason),
            recovery: None,
            source_label: Some(excluded.source_label.clone()),
        };
        let chat = ref_by_label.get(&excluded.source_label);
        let holder = out.iter_mut().find(|candidate| {
            candidate.kind == CandidateKind::Conversation && chat == Some(&candidate_key(candidate))
        });
        match holder {
            Some(candidate) => candidate.diagnostics.push(diagnostic),
            None => {
                let main_label = excluded.source_label.split(" > ").next().unwrap_or_default();
                if let Some(artifact) = artifacts.iter().find(|artifact| artifact.source_label == main_label) {
                    let mut stand_in = unsupported_candidate(artifact, discovery, &diagnostic.message);
                    stand_in.title = excluded.source_label.clone();
                    stand_in.diagnostics = vec![diagnostic];
                    out.push(stand_in);
                }
            }
        }
    }
}

fn preview_conversations(
    approved_roots: &[PathBuf],
    artifacts: &[&DiscoveredArtifact],
    discovery: &DiscoveryResult,
) -> Result<Vec<ImportCandidate>, BridgeError> {
    use crate::import_bundle::{self, BundleSource, TransferBundle};
    if artifacts.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut bundle = TransferBundle::new(BundleSource {
        tool: PROVIDER.into(),
        tool_version: discovery.source_version.clone(),
        schema_version: Some(TRANSCRIPT_GATE.into()),
        adapter_version: ADAPTER_VERSION.into(),
        roots: discovery.approved_roots.clone(),
        detected_at: discovery.discovered_at.clone(),
    });
    let mut skipped_by_ref: std::collections::HashMap<ChatKey, Vec<ImportDiagnostic>> =
        std::collections::HashMap::new();
    let mut ref_by_label: std::collections::HashMap<String, ChatKey> = std::collections::HashMap::new();
    for artifact in artifacts {
        let read = read_artifact_session(approved_roots, artifact, discovery);
        match read {
            Ok(session) => {
                let main_ref = session.chats.first().map(|item| chat_key(&item.chat));
                let lost: Vec<ImportDiagnostic> =
                    session.failures.iter().map(subagent_failure_diagnostic).collect();
                let legacy = legacy_identity(approved_roots, artifact, discovery);
                for mut item in session.chats {
                    if item.chat.agent_role.is_none() {
                        if let Some((native_id, content_hash)) = legacy.clone() {
                            item.chat.legacy_native_id = Some(native_id);
                            item.chat.legacy_content_hash = content_hash;
                        }
                    }
                    // An inline subagent shares its parent's file: key by the
                    // chat itself and add, so neither overwrites the other.
                    skipped_by_ref.entry(chat_key(&item.chat)).or_default().extend(skipped_diagnostics(&item));
                    ref_by_label.insert(item.chat.source_label.clone(), chat_key(&item.chat));
                    bundle.chats.push(item.chat);
                }
                if let Some(main_ref) = main_ref {
                    skipped_by_ref.entry(main_ref).or_default().extend(lost);
                }
            }
            Err(error) => out.push(unsupported_candidate(artifact, discovery, &error.to_string())),
        }
    }
    let lowered = import_bundle::lower(&import_bundle::resolve(vec![bundle])?);
    for mut candidate in lowered.candidates {
        if let Some(diagnostics) = skipped_by_ref.remove(&candidate_key(&candidate)) {
            candidate.diagnostics.extend(diagnostics);
        }
        out.push(candidate);
    }
    attach_exclusions(&mut out, lowered.excluded, &ref_by_label, artifacts, discovery);
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn collect_markdown(
    root: &Path,
    approved_roots: &[PathBuf],
    kind: CandidateKind,
    classification: SourceClassification,
    stability: Stability,
    gate: Option<&str>,
    artifacts: &mut Vec<DiscoveredArtifact>,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> Result<(), BridgeError> {
    let skills_only = kind == CandidateKind::Skill;
    collect_named_files(
        root,
        approved_roots,
        |path| {
            path.extension().and_then(|part| part.to_str()) == Some("md")
                && (!skills_only
                    || path.file_name().and_then(|part| part.to_str()) == Some("SKILL.md"))
        },
        kind,
        classification,
        stability,
        gate,
        artifacts,
        diagnostics,
    )
}

#[allow(clippy::too_many_arguments)]
fn collect_named_files(
    root: &Path,
    approved_roots: &[PathBuf],
    include: impl Fn(&Path) -> bool,
    kind: CandidateKind,
    classification: SourceClassification,
    stability: Stability,
    gate: Option<&str>,
    artifacts: &mut Vec<DiscoveredArtifact>,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> Result<(), BridgeError> {
    if !root.exists() {
        return Ok(());
    }
    let mut stack = vec![(root.to_path_buf(), 0_usize)];
    let mut visited = HashSet::new();
    while let Some((path, depth)) = stack.pop() {
        if depth > MAX_DISCOVERY_DEPTH {
            diagnostics.push(unsupported_diagnostic(
                "discovery_depth_exceeded",
                "Nested Claude source exceeded the safe discovery depth",
                safe_label(&path, approved_roots),
            ));
            continue;
        }
        let canonical = match canonicalize_approved_path(approved_roots, &path) {
            Ok(path) => path,
            Err(_) => {
                diagnostics.push(unsupported_diagnostic(
                    "source_outside_approved_root",
                    "A symlink or nested source resolved outside the approved roots",
                    safe_label(&path, approved_roots),
                ));
                continue;
            }
        };
        if canonical.is_dir() {
            if !visited.insert(canonical.clone()) {
                continue;
            }
            let mut children = fs::read_dir(&canonical)?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .collect::<Vec<_>>();
            children.sort();
            for child in children.into_iter().rev() {
                stack.push((child, depth + 1));
            }
        } else if canonical.is_file() && include(&canonical) {
            push_artifact(
                &canonical,
                approved_roots,
                kind.clone(),
                classification.clone(),
                stability.clone(),
                gate,
                artifacts,
            )?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn push_artifact(
    path: &Path,
    approved_roots: &[PathBuf],
    kind: CandidateKind,
    classification: SourceClassification,
    stability: Stability,
    gate: Option<&str>,
    artifacts: &mut Vec<DiscoveredArtifact>,
) -> Result<(), BridgeError> {
    let canonical = canonicalize_approved_path(approved_roots, path)?;
    let metadata = fs::metadata(&canonical)?;
    let source_label = safe_label(&canonical, approved_roots).unwrap_or_else(|| {
        canonical
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Claude source")
            .to_string()
    });
    let artifact_id = deterministic_identity(
        PROVIDER,
        &canonical.to_string_lossy(),
        Some(&source_label),
        &metadata.len().to_string(),
        &kind,
    );
    artifacts.push(DiscoveredArtifact {
        artifact_id,
        canonical_source_ref: canonical.to_string_lossy().into_owned(),
        source_label,
        kind,
        classification,
        stability,
        estimated_bytes: metadata.len(),
        modified_at: metadata
            .modified()
            .ok()
            .map(|time| DateTime::<Utc>::from(time).to_rfc3339()),
        required_schema_gate: gate.map(str::to_owned),
        title: None,
        project_path: None,
        started_at: None,
    });
    Ok(())
}

fn preview_markdown(
    path: &Path,
    artifact: &DiscoveredArtifact,
    discovery: &DiscoveryResult,
    kind: CandidateKind,
) -> Result<ImportCandidate, BridgeError> {
    let body = fs::read_to_string(path)
        .map_err(|error| BridgeError::Invalid(format!("Claude Markdown is unreadable: {error}")))?;
    if body.trim().is_empty() {
        return Err(BridgeError::Invalid(
            "Empty Claude Markdown is not an import candidate".into(),
        ));
    }
    let payload = if kind == CandidateKind::Memory {
        json!({
            "body": body,
            "memoryType": if path.file_name().and_then(|name| name.to_str()) == Some("MEMORY.md") { "index" } else { "topic" },
            "proposedScope": Value::Null,
            "activationState": "requires_scope_decision"
        })
    } else {
        json!({
            "body": body,
            "activationState": "disabled",
            "sourceKind": kind.as_str()
        })
    };
    candidate(path, artifact, discovery, kind, payload, None)
}

fn preview_settings(
    path: &Path,
    artifact: &DiscoveredArtifact,
    discovery: &DiscoveryResult,
) -> Result<Vec<ImportCandidate>, BridgeError> {
    let value = read_json(path, "Claude settings")?;
    let object = value
        .as_object()
        .ok_or_else(|| BridgeError::Invalid("Claude settings must be a JSON object".into()))?;
    let mut candidates = Vec::new();
    if let Some(hooks) = object.get("hooks") {
        candidates.push(candidate(
            path,
            artifact,
            discovery,
            CandidateKind::Hook,
            json!({"configuration": hooks, "activationState": "disabled"}),
            Some("hooks"),
        )?);
    }
    if let Some(servers) = object.get("mcpServers") {
        candidates.extend(mcp_candidates(path, artifact, discovery, servers)?);
    }
    let reusable: Map<String, Value> = object
        .iter()
        .filter(|(key, _)| !matches!(key.as_str(), "hooks" | "mcpServers" | "env"))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if !reusable.is_empty() {
        candidates.push(candidate(
            path,
            artifact,
            discovery,
            CandidateKind::Prompt,
            json!({"configuration": reusable, "activationState": "disabled"}),
            Some("settings"),
        )?);
    }
    Ok(candidates)
}

fn preview_mcp(
    path: &Path,
    artifact: &DiscoveredArtifact,
    discovery: &DiscoveryResult,
) -> Result<Vec<ImportCandidate>, BridgeError> {
    let value = read_json(path, "Claude MCP configuration")?;
    let servers = value.get("mcpServers").ok_or_else(|| {
        BridgeError::Invalid("Claude MCP configuration has no mcpServers object".into())
    })?;
    mcp_candidates(path, artifact, discovery, servers)
}

fn mcp_candidates(
    path: &Path,
    artifact: &DiscoveredArtifact,
    discovery: &DiscoveryResult,
    servers: &Value,
) -> Result<Vec<ImportCandidate>, BridgeError> {
    let servers = servers
        .as_object()
        .ok_or_else(|| BridgeError::Invalid("mcpServers must be a JSON object".into()))?;
    servers
        .iter()
        .map(|(name, configuration)| {
            candidate(
                path,
                artifact,
                discovery,
                CandidateKind::McpServer,
                json!({
                    "name": name,
                    "configuration": configuration,
                    "activationState": "disabled"
                }),
                Some(name),
            )
        })
        .collect()
}

/// Real Claude Code transcripts interleave `user`/`assistant` turns with
/// bookkeeping records Bridge has no destination for. These are expected,
/// documented shapes (`session_titles.rs` already treats `summary` and
/// `custom-title` as normal Claude output) — skipped silently, not treated as
/// an anomaly worth a diagnostic.
fn is_ignorable_transcript_record(record_type: &str) -> bool {
    matches!(
        record_type,
        "summary"
            | "custom-title"
            | "ai-title"
            | "mode"
            | "pr-link"
            | "queue-operation"
            | "system"
            | "last-prompt"
            | "attachment"
    )
}

fn bump(skipped: &mut BTreeMap<String, u32>, reason: &str) {
    *skipped.entry(reason.to_owned()).or_insert(0) += 1;
}

fn preview_transcript(
    path: &Path,
    artifact: &DiscoveredArtifact,
    discovery: &DiscoveryResult,
) -> Result<ImportCandidate, BridgeError> {
    let content = fs::read_to_string(path).map_err(|error| {
        BridgeError::Invalid(format!("Claude transcript is unreadable: {error}"))
    })?;
    let mut seen_ids = HashSet::new();
    let mut messages = Vec::new();
    let mut project_hint = None;
    let mut skipped: BTreeMap<String, u32> = BTreeMap::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            bump(&mut skipped, "malformed_line");
            continue;
        };
        let Some(object) = record.as_object() else {
            bump(&mut skipped, "non_object_line");
            continue;
        };
        let Some(record_type) = object.get("type").and_then(Value::as_str) else {
            bump(&mut skipped, "untyped_record");
            continue;
        };
        if !matches!(record_type, "user" | "assistant") {
            if !is_ignorable_transcript_record(record_type) {
                bump(&mut skipped, record_type);
            }
            continue;
        }
        project_hint =
            project_hint.or_else(|| object.get("cwd").and_then(Value::as_str).map(str::to_owned));
        let Some((source_id, message)) =
            parse_transcript_record(object, record_type, messages.len() + 1, &mut skipped)
        else {
            continue;
        };
        if !seen_ids.insert(source_id) {
            bump(&mut skipped, "duplicate_source_id");
            continue;
        }
        messages.push(message);
    }
    if messages.is_empty() {
        return Err(BridgeError::Invalid(
            "Claude transcript contains no supported messages".into(),
        ));
    }
    let started_at = messages
        .first()
        .and_then(|message| message.get("timestamp"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let ended_at = messages
        .last()
        .and_then(|message| message.get("timestamp"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let source_native_id = messages
        .first()
        .and_then(|message| message.get("sourceId"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let payload = json!({
        "messages": messages,
        "projectHint": project_hint,
        "historical": true,
        "resumable": false,
        "sourceBadge": "Imported from Claude Code",
        "transcriptParserVersion": LEGACY_TRANSCRIPT_GATE
    });
    let mut result = candidate(
        path,
        artifact,
        discovery,
        CandidateKind::Conversation,
        payload,
        source_native_id.as_deref(),
    )?;
    result.title = result.normalized_payload["messages"]
        .as_array()
        .and_then(|messages| messages.iter().find(|message| message["role"] == "user"))
        .and_then(|message| message["text"].as_str())
        .filter(|text| !text.trim().is_empty())
        .map(|text| {
            text.lines()
                .next()
                .unwrap_or("Imported Claude conversation")
                .chars()
                .take(80)
                .collect::<String>()
        })
        .unwrap_or_else(|| "Imported Claude conversation".into());
    result.created_at = started_at;
    result.updated_at = ended_at;
    result.project_hint = result.normalized_payload["projectHint"]
        .as_str()
        .map(str::to_owned);
    result.diagnostics = skipped
        .into_iter()
        .map(|(kind, count)| ImportDiagnostic {
            code: "transcript_record_skipped".into(),
            severity: "info".into(),
            classification: SourceClassification::Documented,
            message: format!(
                "Skipped {count} '{kind}' record(s); only user/assistant turns are imported"
            ),
            recovery: None,
            source_label: Some(artifact.source_label.clone()),
        })
        .collect();
    Ok(result)
}

/// Parses one `user`/`assistant` transcript record into its imported message
/// shape. Anything malformed about this *specific* record — a missing id or
/// timestamp, an unparsable timestamp, a role that disagrees with the record
/// type — is recorded in `skipped` and the record is dropped, rather than
/// failing the whole transcript over one bad line.
fn parse_transcript_record(
    object: &Map<String, Value>,
    record_type: &str,
    sequence: usize,
    skipped: &mut BTreeMap<String, u32>,
) -> Option<(String, Value)> {
    let Some(source_id) = object.get("uuid").and_then(Value::as_str) else {
        bump(skipped, "missing_uuid");
        return None;
    };
    let Some(timestamp) = object.get("timestamp").and_then(Value::as_str) else {
        bump(skipped, "missing_timestamp");
        return None;
    };
    if DateTime::parse_from_rfc3339(timestamp).is_err() {
        bump(skipped, "invalid_timestamp");
        return None;
    }
    let Some(message) = object.get("message").and_then(Value::as_object) else {
        bump(skipped, "missing_message_payload");
        return None;
    };
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or(record_type);
    if role != record_type {
        bump(skipped, "role_mismatch");
        return None;
    }
    let (text, tool_metadata) = parse_message_content(message.get("content"), record_type, skipped);
    Some((
        source_id.to_owned(),
        json!({
            "sourceId": source_id,
            "role": role,
            "kind": if role == "user" { "user.message" } else { "assistant.message" },
            "text": text,
            "toolMetadata": tool_metadata,
            "timestamp": timestamp,
            "sequence": sequence
        }),
    ))
}

/// Content blocks Bridge does not carry into imported text — reasoning
/// traces and inline images are known, expected shapes in a real transcript,
/// dropped rather than surfaced as an anomaly. Anything else unrecognized is
/// dropped too, but counted, so one exotic block does not cost the whole
/// message.
fn parse_message_content(
    content: Option<&Value>,
    role: &str,
    skipped: &mut BTreeMap<String, u32>,
) -> (String, Vec<Value>) {
    match content {
        Some(Value::String(text)) => (text.clone(), Vec::new()),
        Some(Value::Array(blocks)) => {
            let mut text = Vec::new();
            let mut tools = Vec::new();
            for block in blocks {
                let Some(object) = block.as_object() else {
                    bump(skipped, "non_object_content_block");
                    continue;
                };
                match object.get("type").and_then(Value::as_str) {
                    Some("text") => match object.get("text").and_then(Value::as_str) {
                        Some(value) => text.push(value.to_string()),
                        None => bump(skipped, "text_block_missing_text"),
                    },
                    Some("tool_use") if role == "assistant" => tools.push(json!({
                        "type": "tool_use",
                        "sourceId": object.get("id"),
                        "name": object.get("name")
                    })),
                    Some("tool_result") if role == "user" => tools.push(json!({
                        "type": "tool_result",
                        "sourceId": object.get("tool_use_id"),
                        "isError": object.get("is_error").and_then(Value::as_bool).unwrap_or(false)
                    })),
                    Some("thinking") | Some("image") => {}
                    Some(other) => bump(skipped, &format!("content_block:{other}")),
                    None => bump(skipped, "untyped_content_block"),
                }
            }
            (text.join("\n\n"), tools)
        }
        _ => {
            bump(skipped, "unsupported_content_shape");
            (String::new(), Vec::new())
        }
    }
}

fn candidate(
    path: &Path,
    artifact: &DiscoveredArtifact,
    discovery: &DiscoveryResult,
    kind: CandidateKind,
    payload: Value,
    native_suffix: Option<&str>,
) -> Result<ImportCandidate, BridgeError> {
    let (safe_payload, redaction_summary) = sanitize_import_payload(&payload);
    let hash = content_hash(&safe_payload);
    // `canonical_source_ref` (below) is already the file's stable absolute
    // path, so identity only needs a native suffix to disambiguate multiple
    // candidates carved out of one file (e.g. several MCP servers in one
    // `.mcp.json`). Falling back to the root-relative `source_label` here
    // would make identity depend on which approved root the user picked,
    // breaking dedup and revision detection for the same file re-approved
    // under a different root.
    let source_native_id = native_suffix.map(str::to_owned);
    let schema_version = artifact
        .required_schema_gate
        .as_ref()
        .and_then(|gate| discovery.format_versions.get(gate))
        .cloned();
    let source = source_now(
        PROVIDER,
        ADAPTER_VERSION,
        path,
        discovery.source_version.clone(),
        schema_version,
    );
    let candidate_id = deterministic_identity(
        PROVIDER,
        &source.canonical_source_ref,
        source_native_id.as_deref(),
        &hash,
        &kind,
    );
    Ok(ImportCandidate {
        candidate_id,
        source_native_id,
        source,
        kind: kind.clone(),
        title: title_for(path, &kind, native_suffix),
        created_at: artifact.modified_at.clone(),
        updated_at: artifact.modified_at.clone(),
        project_hint: None,
        content_hash: hash,
        stability: artifact.stability.clone(),
        confidence_bps: if matches!(artifact.stability, Stability::Stable) {
            10000
        } else {
            8000
        },
        selected_by_default: !matches!(kind, CandidateKind::Memory | CandidateKind::Conversation)
            && !kind.is_setup(),
        redaction_summary,
        diagnostics: Vec::new(),
        normalized_payload: safe_payload,
    })
}

fn title_for(path: &Path, kind: &CandidateKind, suffix: Option<&str>) -> String {
    if let Some(suffix) = suffix {
        return format!("{}: {suffix}", kind.as_str().replace('_', " "));
    }
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(kind.as_str())
        .to_string()
}

fn read_json(path: &Path, label: &str) -> Result<Value, BridgeError> {
    let content = fs::read_to_string(path)
        .map_err(|error| BridgeError::Invalid(format!("{label} is unreadable: {error}")))?;
    serde_json::from_str(&content)
        .map_err(|_| BridgeError::Invalid(format!("{label} is malformed; nothing was imported")))
}

fn enforce_size(path: &Path) -> Result<(), BridgeError> {
    let bytes = fs::metadata(path)?.len();
    if bytes > MAX_SOURCE_BYTES {
        return Err(BridgeError::Invalid(format!(
            "Claude source exceeds the {} byte local preview limit",
            MAX_SOURCE_BYTES
        )));
    }
    Ok(())
}

fn safe_label(path: &Path, approved_roots: &[PathBuf]) -> Option<String> {
    approved_roots.iter().find_map(|root| {
        path.strip_prefix(root)
            .ok()
            .map(|relative| relative.to_string_lossy().into_owned())
    })
}

fn unsupported_diagnostic(
    code: &str,
    message: &str,
    source_label: impl Into<Option<String>>,
) -> ImportDiagnostic {
    ImportDiagnostic {
        code: code.into(),
        severity: "warning".into(),
        classification: SourceClassification::UnsupportedPrivate,
        message: message.into(),
        recovery: Some("Choose a documented export or a supported version-gated source".into()),
        source_label: source_label.into(),
    }
}

/// Where this daemon's Claude Code keeps its config: the home folder and
/// `CLAUDE_CONFIG_DIR`. Detection and discovery both read it from here, so a
/// folder detection advertises is one discovery treats as a config home.
pub fn claude_env() -> Option<(PathBuf, Option<PathBuf>)> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    Some((home, std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from)))
}

/// The config homes detection reports on this machine.
pub fn machine_config_roots() -> Vec<PathBuf> {
    claude_env()
        .map(|(home, config_dir)| config_roots(&home, config_dir.as_deref(), &[]))
        .unwrap_or_default()
}

/// Every Claude Code config folder on this machine: `~/.claude`, the folder
/// named by `CLAUDE_CONFIG_DIR`, and each provider account's folder. Missing
/// folders are left out and the same folder reached twice is listed once.
pub fn config_roots(home: &Path, config_dir_env: Option<&Path>, account_homes: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut roots = Vec::new();
    let candidates = std::iter::once(home.join(".claude"))
        .chain(config_dir_env.map(Path::to_path_buf))
        .chain(account_homes.iter().cloned());
    for candidate in candidates {
        let Ok(real) = candidate.canonicalize() else {
            continue;
        };
        if real.is_dir() && seen.insert(real.clone()) {
            roots.push(real);
        }
    }
    roots
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::io::Write;

    fn request(root: &Path) -> DiscoveryRequest {
        DiscoveryRequest {
            provider: PROVIDER.into(),
            approved_roots: vec![root.to_string_lossy().into_owned()],
            selected_export: None,
            source_version: Some("2.1.59".into()),
            schema_version: None,
            format_versions: BTreeMap::from([
                (AUTO_MEMORY_GATE.into(), AUTO_MEMORY_GATE.into()),
                (TRANSCRIPT_GATE.into(), TRANSCRIPT_GATE.into()),
                (SETTINGS_GATE.into(), SETTINGS_GATE.into()),
            ]),
        }
    }

    fn write(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut file = fs::File::create(path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
    }

    fn copy_dir(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap().flatten() {
            let target = to.join(entry.file_name());
            if entry.path().is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), &target).unwrap();
            }
        }
    }

    fn fixture_v2() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testing/fixtures/import/claude/v2/demo")
    }

    /// A real Claude Code 2.1.325 session (sanitized): an Explore subagent,
    /// signature-only thinking, tool calls, a failing read and an image result.
    #[test]
    fn a_real_session_imports_with_its_subagent_tools_thinking_and_image() {
        let root = tempfile::tempdir().unwrap();
        let claude = root.path().join(".claude");
        copy_dir(&fixture_v2(), &claude);
        let discovery = ClaudeCodeImporter.discover(&request(&claude)).unwrap();
        let conversations: Vec<_> = discovery
            .artifacts
            .iter()
            .filter(|item| item.kind == CandidateKind::Conversation)
            .collect();
        assert_eq!(conversations.len(), 1, "the subagent file is part of its session, not a session of its own");
        let selection = DiscoverySelection { artifact_ids: conversations.iter().map(|item| item.artifact_id.clone()).collect() };
        let preview = ClaudeCodeImporter.preview(&discovery, &selection).unwrap();
        let chats: Vec<_> = preview.iter().filter(|c| c.kind == CandidateKind::Conversation).collect();
        assert_eq!(chats.len(), 2, "main chat and its Explore subagent");
        let normalized: Vec<_> = chats.iter().map(|c| ClaudeCodeImporter.normalize((*c).clone()).unwrap()).collect();
        let plan = crate::external_import::ImportPlan {
            selected_candidate_ids: chats.iter().map(|c| c.candidate_id.clone()).collect(),
            conflict_policy: crate::external_import::ConflictPolicy::Skip,
            setup_activation_policy: crate::external_import::SetupActivationPolicy::Disabled,
            memory_scope: None,
            dry_run: false,
            created_at: "2026-11-17T00:00:00Z".into(),
        };
        let db = crate::store::open(Path::new(":memory:")).unwrap();
        let commit = crate::external_import::commit_import(&db, &normalized, &plan).unwrap();
        assert_eq!(commit.imported, 2, "{commit:?}");
        let (harness, label, cwd): (String, String, Option<String>) = db
            .query_row("SELECT harness,label,cwd FROM sessions WHERE parent_session_id IS NULL", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap();
        assert_eq!((harness.as_str(), label.as_str(), cwd.as_deref()), ("claude", "Imported from Claude Code", Some("/work/invoice-app")));
        let child_title: String = db.query_row("SELECT title FROM sessions WHERE parent_session_id IS NOT NULL", [], |r| r.get(0)).unwrap();
        assert!(child_title.starts_with("Explore: "), "{child_title}");
        // Parity: every item the reader saw is a stored entry.
        let main_path = PathBuf::from(&conversations[0].canonical_source_ref);
        let read = transcript::read_session(&main_path, "x", &[claude.canonicalize().unwrap()]).unwrap().chats;
        let expected: i64 = read.iter().map(|item| item.chat.entries.len() as i64).sum();
        let stored: i64 = db.query_row("SELECT COUNT(*) FROM session_entries", [], |r| r.get(0)).unwrap();
        assert_eq!(stored, expected);
        let kinds: Vec<String> = db
            .prepare("SELECT e.kind FROM session_entries e JOIN sessions s ON s.id=e.session_id WHERE s.parent_session_id IS NULL ORDER BY e.sequence")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for kind in ["user.message", "tool.completed", "assistant.message"] {
            assert!(kinds.iter().any(|k| k == kind), "{kind} missing from {kinds:?}");
        }
        // Claude Code 2.1.325 keeps thinking as an empty, signature-only block:
        // there is no text to show, so none is imported (and none is invented).
        assert!(!kinds.iter().any(|k| k == "reasoning"));
        let golden_path = fixture_v2().join("golden-kinds.txt");
        let actual = kinds.join("\n") + "\n";
        if std::env::var_os("BRIDGE_UPDATE_GOLDEN").is_some() {
            fs::write(&golden_path, &actual).unwrap();
        }
        assert_eq!(fs::read_to_string(&golden_path).unwrap(), actual, "entry sequence drifted from the golden file");
        let image_read: i64 = db.query_row("SELECT COUNT(*) FROM session_entries WHERE kind='tool.completed' AND payload LIKE '%[image]%'", [], |r| r.get(0)).unwrap();
        assert!(image_read >= 1, "the logo read keeps an [image] marker");
        // Idempotent: the same import again writes nothing.
        let again = crate::external_import::commit_import(&db, &normalized, &plan).unwrap();
        assert_eq!((again.imported, again.skipped), (0, 2));
    }

    fn preview_all(claude: &Path) -> Vec<ImportCandidate> {
        let discovery = ClaudeCodeImporter.discover(&request(claude)).unwrap();
        let ids = discovery
            .artifacts
            .iter()
            .filter(|item| item.kind == CandidateKind::Conversation)
            .map(|item| item.artifact_id.clone())
            .collect();
        ClaudeCodeImporter.preview(&discovery, &DiscoverySelection { artifact_ids: ids }).unwrap()
    }

    fn session_lines(session: &str, text: &str) -> String {
        [
            json!({"type":"user","uuid":format!("{session}-u"),"sessionId":session,"timestamp":"2026-11-01T10:00:00Z","cwd":"/work/app","message":{"role":"user","content":text}}).to_string(),
            json!({"type":"assistant","uuid":format!("{session}-a"),"sessionId":session,"timestamp":"2026-11-01T10:00:01Z","message":{"role":"assistant","content":[{"type":"text","text":"ok"}]}}).to_string(),
        ]
        .join("\n")
    }

    #[test]
    fn a_subagent_symlink_outside_the_approved_folder_is_not_read() {
        let root = tempfile::tempdir().unwrap();
        let claude = root.path().join(".claude");
        write(&claude.join("projects/p/sess-1.jsonl"), &session_lines("sess-1", "main task"));
        let outside = root.path().join("outside");
        write(&outside.join("private.jsonl"), &session_lines("sess-1", "OUTSIDE-SENTINEL-transcript"));
        write(&outside.join("private.meta.json"), r#"{"agentType":"OUTSIDE-SENTINEL-meta"}"#);
        let subagents = claude.join("projects/p/sess-1/subagents");
        fs::create_dir_all(&subagents).unwrap();
        std::os::unix::fs::symlink(outside.join("private.jsonl"), subagents.join("agent-private.jsonl")).unwrap();
        std::os::unix::fs::symlink(outside.join("private.meta.json"), subagents.join("agent-private.meta.json")).unwrap();
        // An inside child whose meta escapes: the transcript is read, the meta is not.
        write(&subagents.join("agent-inside.jsonl"), &session_lines("sess-1", "inside child"));
        std::os::unix::fs::symlink(outside.join("private.meta.json"), subagents.join("agent-inside.meta.json")).unwrap();
        let preview = preview_all(&claude);
        let encoded = serde_json::to_string(&preview).unwrap();
        assert!(!encoded.contains("OUTSIDE-SENTINEL"), "nothing outside the approved folder is read");
        assert!(encoded.contains("inside child"));
        let diagnostics: Vec<&ImportDiagnostic> = preview.iter().flat_map(|c| c.diagnostics.iter()).collect();
        assert!(
            diagnostics.iter().any(|d| d.code == "excluded_subagent" && d.message.contains("outside the approved folders")),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn an_unreadable_subagent_is_reported() {
        let root = tempfile::tempdir().unwrap();
        let claude = root.path().join(".claude");
        write(&claude.join("projects/p/sess-1.jsonl"), &session_lines("sess-1", "main task"));
        let huge = claude.join("projects/p/sess-1/subagents/agent-huge.jsonl");
        fs::create_dir_all(huge.parent().unwrap()).unwrap();
        fs::File::create(&huge).unwrap().set_len(transcript::MAX_TRANSCRIPT_BYTES + 1).unwrap();
        let preview = preview_all(&claude);
        let chats: Vec<_> = preview.iter().filter(|c| c.kind == CandidateKind::Conversation).collect();
        assert_eq!(chats.len(), 1, "the readable session still imports");
        assert!(
            chats[0].diagnostics.iter().any(|d| d.code == "excluded_subagent" && d.message.contains("agent-huge") && d.message.contains("could not be read")),
            "{:?}",
            chats[0].diagnostics
        );
    }

    fn commit_all(db: &rusqlite::Connection, candidates: &[ImportCandidate]) -> crate::external_import::ImportCommit {
        let normalized: Vec<_> = candidates.iter().map(|c| ClaudeCodeImporter.normalize(c.clone()).unwrap()).collect();
        crate::external_import::commit_import(
            db,
            &normalized,
            &crate::external_import::ImportPlan {
                selected_candidate_ids: candidates.iter().map(|c| c.candidate_id.clone()).collect(),
                conflict_policy: crate::external_import::ConflictPolicy::Skip,
                setup_activation_policy: crate::external_import::SetupActivationPolicy::Disabled,
                memory_scope: None,
                dry_run: false,
                created_at: "2026-11-18T00:00:00Z".into(),
            },
        )
        .unwrap()
    }

    #[test]
    fn a_chat_imported_before_the_upgrade_is_not_imported_twice() {
        let root = tempfile::tempdir().unwrap();
        let claude = root.path().join(".claude");
        let path = claude.join("projects/p/sess-1.jsonl");
        write(&path, &session_lines("sess-1", "imported by the old reader"));
        let db = crate::store::open(Path::new(":memory:")).unwrap();
        // What the v1 reader wrote: one chat keyed by its first message.
        let discovery = ClaudeCodeImporter.discover(&request(&claude)).unwrap();
        let artifact = discovery.artifacts.iter().find(|a| a.kind == CandidateKind::Conversation).unwrap().clone();
        let v1 = preview_transcript(&path.canonicalize().unwrap(), &artifact, &discovery).unwrap();
        assert_eq!(commit_all(&db, &[v1]).imported, 1);
        // The same file through v2 is recognised, and the native mapping is filled in.
        let v2: Vec<_> = preview_all(&claude).into_iter().filter(|c| c.kind == CandidateKind::Conversation).collect();
        let again = commit_all(&db, &v2);
        assert_eq!((again.imported, again.skipped), (0, 1), "{again:?}");
        let mapped: i64 = db.query_row("SELECT COUNT(*) FROM import_native_sessions WHERE native_key='claude:sess-1'", [], |r| r.get(0)).unwrap();
        assert_eq!(mapped, 1);
        // A changed file asks first instead of importing a second chat.
        let root = tempfile::tempdir().unwrap();
        let claude = root.path().join(".claude");
        let path = claude.join("projects/p/sess-2.jsonl");
        write(&path, &session_lines("sess-2", "first version"));
        let db = crate::store::open(Path::new(":memory:")).unwrap();
        let discovery = ClaudeCodeImporter.discover(&request(&claude)).unwrap();
        let artifact = discovery.artifacts.iter().find(|a| a.kind == CandidateKind::Conversation).unwrap().clone();
        commit_all(&db, &[preview_transcript(&path.canonicalize().unwrap(), &artifact, &discovery).unwrap()]);
        let mut grown = session_lines("sess-2", "first version");
        grown.push('\n');
        grown.push_str(&json!({"type":"user","uuid":"sess-2-later","sessionId":"sess-2","timestamp":"2026-11-02T10:00:00Z","message":{"role":"user","content":"a later turn"}}).to_string());
        write(&path, &grown);
        let v2: Vec<_> = preview_all(&claude).into_iter().filter(|c| c.kind == CandidateKind::Conversation).collect();
        let changed = commit_all(&db, &v2);
        assert_eq!((changed.imported, changed.changed), (0, 1), "{changed:?}");
    }

    /// Import `path` with the retained v1 reader, as an earlier Bridge did.
    fn import_with_v1(db: &rusqlite::Connection, claude: &Path, path: &Path) -> crate::external_import::ImportCommit {
        let discovery = ClaudeCodeImporter.discover(&request(claude)).unwrap();
        let canonical = path.canonicalize().unwrap();
        let artifact = discovery
            .artifacts
            .iter()
            .find(|a| a.kind == CandidateKind::Conversation && Path::new(&a.canonical_source_ref) == canonical)
            .unwrap()
            .clone();
        commit_all(db, &[preview_transcript(&canonical, &artifact, &discovery).unwrap()])
    }

    #[test]
    fn a_v1_import_that_grew_past_ten_mib_is_still_recognised() {
        let root = tempfile::tempdir().unwrap();
        let claude = root.path().join(".claude");
        let path = claude.join("projects/p/sess-big.jsonl");
        write(&path, &session_lines("sess-big", "small when v1 imported it"));
        let db = crate::store::open(Path::new(":memory:")).unwrap();
        assert_eq!(import_with_v1(&db, &claude, &path).imported, 1);
        // Claude keeps appending: the file passes the 10 MiB v1 limit.
        let long = json!({"type":"assistant","uuid":"later","sessionId":"sess-big","timestamp":"2026-11-02T10:00:00Z","message":{"role":"assistant","content":[{"type":"text","text":"x".repeat(MAX_SOURCE_BYTES as usize + 1024)}]}}).to_string();
        std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(format!("\n{long}").as_bytes()).unwrap();
        assert!(fs::metadata(&path).unwrap().len() > MAX_SOURCE_BYTES);
        let v2: Vec<_> = preview_all(&claude).into_iter().filter(|c| c.kind == CandidateKind::Conversation).collect();
        let commit = commit_all(&db, &v2);
        assert_eq!((commit.imported, commit.changed), (0, 1), "{commit:?}");
    }

    #[test]
    fn a_v1_import_of_any_copy_is_recognised() {
        let root = tempfile::tempdir().unwrap();
        let claude = root.path().join(".claude");
        let lines = session_lines("sess-twin", "one session, two copies");
        write(&claude.join("projects/a/sess-twin.jsonl"), &lines);
        write(&claude.join("projects/b/sess-twin.jsonl"), &lines);
        let db = crate::store::open(Path::new(":memory:")).unwrap();
        // v1 imported the copy in b; v2 resolves both and makes a the primary.
        assert_eq!(import_with_v1(&db, &claude, &claude.join("projects/b/sess-twin.jsonl")).imported, 1);
        let v2: Vec<_> = preview_all(&claude).into_iter().filter(|c| c.kind == CandidateKind::Conversation).collect();
        assert_eq!(v2.len(), 1, "the two copies resolve to one chat");
        let commit = commit_all(&db, &v2);
        assert_eq!((commit.imported, commit.skipped), (0, 1), "{commit:?}");
        let chats: i64 = db.query_row("SELECT COUNT(*) FROM sessions WHERE kind='imported'", [], |r| r.get(0)).unwrap();
        assert_eq!(chats, 1);
    }

    #[test]
    fn a_child_of_a_changed_v1_parent_attaches_to_it() {
        let root = tempfile::tempdir().unwrap();
        let claude = root.path().join(".claude");
        let path = claude.join("projects/p/sess-parent.jsonl");
        write(&path, &session_lines("sess-parent", "parent before"));
        let db = crate::store::open(Path::new(":memory:")).unwrap();
        assert_eq!(import_with_v1(&db, &claude, &path).imported, 1);
        let existing: String = db.query_row("SELECT id FROM sessions WHERE kind='imported'", [], |r| r.get(0)).unwrap();
        // The parent grows and gains a subagent.
        let later = json!({"type":"user","uuid":"later","sessionId":"sess-parent","timestamp":"2026-11-02T10:00:00Z","message":{"role":"user","content":"spawn a helper"}}).to_string();
        std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(format!("\n{later}").as_bytes()).unwrap();
        write(&claude.join("projects/p/sess-parent/subagents/agent-h.jsonl"), &session_lines("sess-parent", "helper task"));
        let v2: Vec<_> = preview_all(&claude).into_iter().filter(|c| c.kind == CandidateKind::Conversation).collect();
        let commit = commit_all(&db, &v2);
        assert_eq!((commit.imported, commit.changed), (1, 1), "{commit:?}");
        let (parent, depth): (Option<String>, i64) = db
            .query_row("SELECT parent_session_id, depth FROM sessions WHERE title='helper task'", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!((parent.as_deref(), depth), (Some(existing.as_str()), 1));
    }

    #[test]
    fn a_parent_keeps_its_losses_next_to_an_inline_child() {
        let root = tempfile::tempdir().unwrap();
        let claude = root.path().join(".claude");
        let lines = [
            json!({"type":"user","uuid":"u1","sessionId":"sess-doc","timestamp":"2026-11-01T10:00:00Z","cwd":"/w","message":{"role":"user","content":[{"type":"text","text":"read this pdf"},{"type":"document","source":{"type":"base64","media_type":"application/pdf","data":"JVBERi0="}}]}}).to_string(),
            json!({"type":"user","uuid":"s1","sessionId":"sess-doc","isSidechain":true,"agentId":"abc","timestamp":"2026-11-01T10:00:01Z","message":{"role":"user","content":"summarise the pdf"}}).to_string(),
            json!({"type":"assistant","uuid":"s2","sessionId":"sess-doc","isSidechain":true,"agentId":"abc","timestamp":"2026-11-01T10:00:02Z","message":{"role":"assistant","content":[{"type":"text","text":"done"}]}}).to_string(),
        ];
        write(&claude.join("projects/p/sess-doc.jsonl"), &lines.join("\n"));
        let preview = preview_all(&claude);
        let chats: Vec<_> = preview.iter().filter(|c| c.kind == CandidateKind::Conversation).collect();
        assert_eq!(chats.len(), 2);
        let parent = chats.iter().find(|c| c.source_native_id.as_deref() == Some("sess-doc")).unwrap();
        assert!(
            parent.diagnostics.iter().any(|d| d.message.contains("content:document")),
            "the dropped document is reported on its own chat: {:?}",
            parent.diagnostics
        );
    }

    #[test]
    fn a_custom_config_folder_is_read_as_a_claude_home() {
        let home = tempfile::tempdir().unwrap();
        let custom = home.path().join(".claude-work");
        write(&custom.join("projects/-work-app/sess-9.jsonl"), &session_lines("sess-9", "work account chat"));
        write(&custom.join("projects/-work-app/memory/MEMORY.md"), "- uses pnpm\n");
        let found = crate::import_sources::detect(home.path(), Some(&custom), &[]);
        let claude = &found[0];
        assert_eq!(claude.counts.chats, 1, "detection counts the custom folder's chat");
        // Review discovers exactly what detection counted, from the same roots.
        let request = DiscoveryRequest {
            provider: PROVIDER.into(),
            approved_roots: claude.roots.clone(),
            selected_export: None,
            source_version: None,
            schema_version: None,
            format_versions: BTreeMap::new(),
        };
        let discovery = ClaudeCodeImporter.discover(&request).unwrap();
        let kinds: Vec<&CandidateKind> = discovery.artifacts.iter().map(|a| &a.kind).collect();
        assert_eq!(kinds.iter().filter(|k| ***k == CandidateKind::Conversation).count(), 1, "{kinds:?}");
        assert_eq!(kinds.iter().filter(|k| ***k == CandidateKind::Memory).count(), 1, "{kinds:?}");
    }

    /// A config folder detection found, before Claude Code has saved a
    /// project in it (no `projects/`, then an empty one): review lists the
    /// setup items detection counted.
    #[test]
    fn a_detected_config_home_with_only_setup_is_listed() {
        let home = tempfile::tempdir().unwrap();
        let custom = home.path().join(".claude-work");
        write(&custom.join("settings.json"), r#"{"model":"opus"}"#);
        write(&custom.join("agents/reviewer.md"), "---\nname: reviewer\n---\nReview diffs.\n");
        for state in ["no projects folder", "empty projects folder"] {
            if state == "empty projects folder" {
                fs::create_dir_all(custom.join("projects")).unwrap();
            }
            let claude = &crate::import_sources::detect(home.path(), Some(&custom), &[])[0];
            assert_eq!(claude.counts.setup, 2, "{state}: detection counts the setup");
            let request = DiscoveryRequest {
                provider: PROVIDER.into(),
                approved_roots: claude.roots.clone(),
                selected_export: None,
                source_version: None,
                schema_version: None,
                format_versions: BTreeMap::new(),
            };
            let homes = config_roots(home.path(), Some(&custom), &[]);
            let discovery = ClaudeCodeImporter.discover_with_homes(&request, &homes).unwrap();
            let mut kinds: Vec<String> = discovery.artifacts.iter().map(|a| format!("{:?}", a.kind)).collect();
            kinds.sort();
            assert_eq!(kinds, vec!["Agent", "Prompt"], "{state}: review lists what detection counted");
        }
    }

    /// Only the folders this machine uses as config homes are trusted by
    /// identity: an unrelated folder with the same files is still a project.
    #[test]
    fn an_unrelated_folder_is_not_a_config_home_by_identity() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        write(&repo.join("settings.json"), r#"{"model":"opus"}"#);
        write(&repo.join("agents/reviewer.md"), "agent");
        let other_home = root.path().join(".claude-work");
        fs::create_dir_all(&other_home).unwrap();
        let homes = vec![other_home.canonicalize().unwrap()];
        let discovery = ClaudeCodeImporter.discover_with_homes(&request(&repo), &homes).unwrap();
        assert!(discovery.artifacts.is_empty(), "{:?}", discovery.artifacts);
    }

    #[test]
    fn a_repository_with_a_projects_folder_is_not_mistaken_for_a_claude_home() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("my-repo");
        write(&repo.join("projects/website/notes.jsonl"), "{}");
        write(&repo.join("CLAUDE.md"), "repo rules");
        let discovery = ClaudeCodeImporter.discover(&request(&repo)).unwrap();
        assert!(discovery.artifacts.iter().all(|a| a.kind != CandidateKind::Conversation));
        assert!(discovery.artifacts.iter().any(|a| a.kind == CandidateKind::Instruction));
    }

    #[test]
    fn config_roots_cover_home_env_and_accounts_once_each() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        fs::create_dir_all(home.join(".claude")).unwrap();
        let work = root.path().join("work-account");
        fs::create_dir_all(&work).unwrap();
        let missing = root.path().join("missing");
        let roots = config_roots(&home, Some(&home.join(".claude")), &[work.clone(), missing, work.clone()]);
        assert_eq!(roots, vec![home.join(".claude").canonicalize().unwrap(), work.canonicalize().unwrap()]);
    }

    #[test]
    fn discovers_documented_markdown_inside_approved_roots() {
        let root = tempfile::tempdir().unwrap();
        write(&root.path().join("CLAUDE.md"), "# Project rules");
        write(&root.path().join("CLAUDE.local.md"), "# Local rules");
        write(&root.path().join(".claude/CLAUDE.md"), "# Nested rules");
        write(&root.path().join(".claude/rules/frontend/ui.md"), "# UI");
        write(&root.path().join(".claude/commands/review.md"), "# Review");
        write(&root.path().join(".claude/agents/auditor.md"), "# Auditor");
        write(
            &root.path().join(".claude/skills/test/SKILL.md"),
            "# Test skill",
        );

        let discovery = ClaudeCodeImporter.discover(&request(root.path())).unwrap();
        let labels: Vec<_> = discovery
            .artifacts
            .iter()
            .map(|item| item.source_label.as_str())
            .collect();
        assert!(labels.contains(&"CLAUDE.md"));
        assert!(labels.contains(&"CLAUDE.local.md"));
        assert!(labels.contains(&".claude/CLAUDE.md"));
        assert!(labels.contains(&".claude/rules/frontend/ui.md"));
        assert!(discovery
            .artifacts
            .iter()
            .all(|item| item.classification == SourceClassification::Documented));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_paths_outside_approved_roots() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        write(&outside.path().join("secret.md"), "outside");
        fs::create_dir_all(root.path().join(".claude/rules")).unwrap();
        symlink(
            outside.path().join("secret.md"),
            root.path().join(".claude/rules/escape.md"),
        )
        .unwrap();
        let discovery = ClaudeCodeImporter.discover(&request(root.path())).unwrap();
        assert!(discovery.artifacts.is_empty());
        assert!(discovery
            .diagnostics
            .iter()
            .any(|item| item.code == "source_outside_approved_root"));
    }

    #[test]
    fn gated_memory_preserves_type_provenance_and_scope() {
        let root = tempfile::tempdir().unwrap();
        let claude = root.path().join(".claude");
        let fixture = include_str!("../../../testing/fixtures/import/claude/auto-memory/MEMORY.md");
        write(&claude.join("projects/demo/memory/MEMORY.md"), fixture);
        let discovery = ClaudeCodeImporter.discover(&request(&claude)).unwrap();
        let memory = discovery
            .artifacts
            .iter()
            .find(|item| item.kind == CandidateKind::Memory)
            .unwrap();
        let preview = ClaudeCodeImporter
            .preview(
                &discovery,
                &DiscoverySelection {
                    artifact_ids: vec![memory.artifact_id.clone()],
                },
            )
            .unwrap();
        assert_eq!(
            preview[0].source.schema_version.as_deref(),
            Some(AUTO_MEMORY_GATE)
        );
        assert_eq!(preview[0].normalized_payload["memoryType"], "index");
        assert!(preview[0].normalized_payload["proposedScope"].is_null());
        assert!(!preview[0].selected_by_default);
    }

    #[test]
    fn gated_jsonl_preserves_order_roles_timestamps_and_ids() {
        let root = tempfile::tempdir().unwrap();
        let claude = root.path().join(".claude");
        write(
            &claude.join("projects/demo/session.jsonl"),
            include_str!("../../../testing/fixtures/import/claude/transcripts/simple.jsonl"),
        );
        let discovery = ClaudeCodeImporter.discover(&request(&claude)).unwrap();
        let transcript = discovery
            .artifacts
            .iter()
            .find(|item| item.kind == CandidateKind::Conversation)
            .unwrap();
        let preview = ClaudeCodeImporter
            .preview(
                &discovery,
                &DiscoverySelection {
                    artifact_ids: vec![transcript.artifact_id.clone()],
                },
            )
            .unwrap();
        let messages = preview[0].normalized_payload["messages"]
            .as_array()
            .unwrap();
        // v2 keeps the tool call and its result, between the two user turns.
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["sourceId"], "msg-assistant-1:0");
        assert_eq!(messages[2]["kind"], "tool.completed");
        assert_eq!(messages[2]["data"]["name"], "Read");
        assert_eq!(messages[2]["data"]["aggregatedOutput"], "source omitted");
        assert_eq!(messages[2]["status"], "completed");
        assert_eq!(messages[3]["text"], "Continue with the local-only design.");
        assert_eq!(messages[3]["sequence"], 4);
        assert_eq!(preview[0].project_hint.as_deref(), Some("/safe/project"));
        assert_eq!(
            preview[0].source.schema_version.as_deref(),
            Some(TRANSCRIPT_GATE)
        );
    }

    #[test]
    fn rejects_transcripts_with_nothing_importable_left() {
        for fixture in [
            include_str!("../../../testing/fixtures/import/claude/transcripts/malformed.jsonl"),
            include_str!("../../../testing/fixtures/import/claude/transcripts/unsupported.jsonl"),
        ] {
            let root = tempfile::tempdir().unwrap();
            let claude = root.path().join(".claude");
            write(&claude.join("projects/demo/session.jsonl"), fixture);
            let discovery = ClaudeCodeImporter.discover(&request(&claude)).unwrap();
            let transcript = discovery
                .artifacts
                .iter()
                .find(|item| item.kind == CandidateKind::Conversation)
                .unwrap();
            // The batch itself still succeeds — this artifact just surfaces
            // as an uncommittable, diagnostic-only candidate instead of
            // aborting every other selected artifact.
            let preview = ClaudeCodeImporter
                .preview(
                    &discovery,
                    &DiscoverySelection {
                        artifact_ids: vec![transcript.artifact_id.clone()],
                    },
                )
                .unwrap();
            assert_eq!(preview.len(), 1);
            assert_eq!(preview[0].kind, CandidateKind::Unsupported);
        }
    }

    /// A duplicate source message id drops the second occurrence with a
    /// diagnostic rather than failing the whole transcript: one repeated line
    /// in an otherwise-good export should not cost every other message in it.
    #[test]
    fn duplicate_source_id_drops_the_repeat_and_keeps_the_rest() {
        let root = tempfile::tempdir().unwrap();
        let claude = root.path().join(".claude");
        write(
            &claude.join("projects/demo/session.jsonl"),
            include_str!("../../../testing/fixtures/import/claude/transcripts/duplicate-id.jsonl"),
        );
        let discovery = ClaudeCodeImporter.discover(&request(&claude)).unwrap();
        let transcript = discovery
            .artifacts
            .iter()
            .find(|item| item.kind == CandidateKind::Conversation)
            .unwrap();
        let preview = ClaudeCodeImporter
            .preview(
                &discovery,
                &DiscoverySelection {
                    artifact_ids: vec![transcript.artifact_id.clone()],
                },
            )
            .unwrap();
        assert_eq!(preview.len(), 1);
        let messages = preview[0].normalized_payload["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert!(preview[0]
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("duplicate_source_id")));
    }

    /// The reviewer's core complaint: a real transcript interleaves
    /// `user`/`assistant` turns with metadata records (`ai-title`, `system`,
    /// ...) and messages carrying `thinking`/`image` blocks. None of that
    /// should abort the import — only the unsupported pieces are dropped.
    #[test]
    fn tolerates_interleaved_metadata_records_and_unsupported_content_blocks() {
        let root = tempfile::tempdir().unwrap();
        let claude = root.path().join(".claude");
        let lines = [
            json!({"type":"ai-title","uuid":"t-1","timestamp":"2026-08-01T09:59:59Z","title":"Demo"}).to_string(),
            json!({
                "type":"user","uuid":"u-1","timestamp":"2026-08-01T10:00:00Z","cwd":"/safe/project",
                "message":{"role":"user","content":"Investigate the flaky test"}
            }).to_string(),
            json!({"type":"system","uuid":"s-1","timestamp":"2026-08-01T10:00:00Z","content":"internal note"}).to_string(),
            json!({
                "type":"assistant","uuid":"a-1","timestamp":"2026-08-01T10:00:01Z",
                "message":{"role":"assistant","content":[
                    {"type":"thinking","thinking":"reasoning that must not leak"},
                    {"type":"text","text":"Found it."},
                    {"type":"image","source":{"type":"base64","data":"not-really-image-bytes"}}
                ]}
            }).to_string(),
            json!({"type":"queue-operation","uuid":"q-1","timestamp":"2026-08-01T10:00:02Z"}).to_string(),
            json!({"type":"future-unknown-record","uuid":"f-1","timestamp":"2026-08-01T10:00:03Z"}).to_string(),
        ];
        write(
            &claude.join("projects/demo/session.jsonl"),
            &lines.join("\n"),
        );
        let discovery = ClaudeCodeImporter.discover(&request(&claude)).unwrap();
        let transcript = discovery
            .artifacts
            .iter()
            .find(|item| item.kind == CandidateKind::Conversation)
            .unwrap();
        let preview = ClaudeCodeImporter
            .preview(
                &discovery,
                &DiscoverySelection {
                    artifact_ids: vec![transcript.artifact_id.clone()],
                },
            )
            .unwrap();
        assert_eq!(preview.len(), 1);
        let messages = preview[0].normalized_payload["messages"].as_array().unwrap();
        // v2 imports thinking as a reasoning entry, the way Claude Code showed
        // it; an image block from the assistant is not a shape Claude writes,
        // so it is counted and dropped.
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1]["kind"], "reasoning");
        assert_eq!(messages[1]["text"], "reasoning that must not leak");
        assert_eq!(messages[2]["text"], "Found it.");
        let encoded = serde_json::to_string(&preview[0]).unwrap();
        assert!(!encoded.contains("not-really-image-bytes"));
        assert!(preview[0]
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("content:image")));
        assert!(preview[0]
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("future-unknown-record")));
    }

    #[test]
    fn setup_candidates_are_inert_and_secret_free() {
        let root = tempfile::tempdir().unwrap();
        let secret = ["sk", "ant", "fixture-value-that-must-never-survive"].join("-");
        write(
            &root.path().join(".claude/settings.json"),
            &json!({
                "env": {"ANTHROPIC_API_KEY": secret},
                "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "echo done"}]}]},
                "model": "sonnet"
            })
            .to_string(),
        );
        write(
            &root.path().join(".mcp.json"),
            &json!({
                "mcpServers": {"local": {"command": "server", "env": {"TOKEN": secret}}}
            })
            .to_string(),
        );
        let discovery = ClaudeCodeImporter.discover(&request(root.path())).unwrap();
        let preview = ClaudeCodeImporter
            .preview(
                &discovery,
                &DiscoverySelection {
                    artifact_ids: discovery
                        .artifacts
                        .iter()
                        .map(|item| item.artifact_id.clone())
                        .collect(),
                },
            )
            .unwrap();
        assert!(preview.iter().any(|item| item.kind == CandidateKind::Hook));
        assert!(preview
            .iter()
            .any(|item| item.kind == CandidateKind::McpServer));
        assert!(preview
            .iter()
            .filter(|item| item.kind.is_setup())
            .all(|item| !item.selected_by_default));
        let encoded = serde_json::to_string(&preview).unwrap();
        assert!(!encoded.contains(&secret));
        assert!(!encoded.contains("ANTHROPIC_API_KEY"));
        assert!(encoded.contains("disabled"));
    }

    #[test]
    fn unknown_private_format_gate_fails_before_reading_content() {
        let root = tempfile::tempdir().unwrap();
        let claude = root.path().join(".claude");
        write(
            &claude.join("projects/demo/session.jsonl"),
            include_str!("../../../testing/fixtures/import/claude/transcripts/simple.jsonl"),
        );
        // `discover()` stamps `format_versions` from this build's own known-
        // supported map — a wire caller cannot influence it (that tautology is
        // exactly what let a forged discovery bypass the gate before). To
        // simulate a future artifact gated on a version this build never
        // learned about, mutate the *returned* discovery rather than the
        // request.
        let mut discovery = ClaudeCodeImporter.discover(&request(&claude)).unwrap();
        discovery
            .format_versions
            .insert(TRANSCRIPT_GATE.into(), "future-private-v9".into());
        let transcript = discovery
            .artifacts
            .iter()
            .find(|item| item.kind == CandidateKind::Conversation)
            .unwrap();
        // A rejected artifact no longer aborts the whole preview batch — it
        // surfaces as a diagnostic-only, uncommittable candidate instead, so
        // one gate rejection cannot blank out everything else selected in the
        // same call.
        let preview = ClaudeCodeImporter
            .preview(
                &discovery,
                &DiscoverySelection {
                    artifact_ids: vec![transcript.artifact_id.clone()],
                },
            )
            .unwrap();
        assert_eq!(preview.len(), 1);
        assert_eq!(preview[0].kind, CandidateKind::Unsupported);
        assert!(preview[0]
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("not allowlisted")));
    }

    #[test]
    fn selected_export_needs_no_directory_discovery_root() {
        let root = tempfile::tempdir().unwrap();
        let export = root.path().join("session.jsonl");
        write(
            &export,
            include_str!("../../../testing/fixtures/import/claude/transcripts/simple.jsonl"),
        );
        let mut request = request(root.path());
        request.approved_roots.clear();
        request.selected_export = Some(export.to_string_lossy().into_owned());
        let discovery = ClaudeCodeImporter.discover(&request).unwrap();
        assert_eq!(discovery.artifacts.len(), 1);
        assert_eq!(discovery.artifacts[0].kind, CandidateKind::Conversation);
    }

    #[test]
    fn transcript_title_and_project_hint_use_only_redacted_values() {
        let root = tempfile::tempdir().unwrap();
        let claude = root.path().join(".claude");
        let secret = ["sk", "ant", "title-project-secret-with-enough-entropy"].join("-");
        write(
            &claude.join("projects/demo/session.jsonl"),
            &json!({
                "type": "user",
                "uuid": "message-1",
                "timestamp": "2026-08-01T10:00:00Z",
                "cwd": format!("/project/{secret}"),
                "message": {"role": "user", "content": format!("Use {secret}")}
            })
            .to_string(),
        );
        let discovery = ClaudeCodeImporter.discover(&request(&claude)).unwrap();
        let transcript = discovery
            .artifacts
            .iter()
            .find(|item| item.kind == CandidateKind::Conversation)
            .unwrap();
        let preview = ClaudeCodeImporter
            .preview(
                &discovery,
                &DiscoverySelection {
                    artifact_ids: vec![transcript.artifact_id.clone()],
                },
            )
            .unwrap();
        assert!(!preview[0].title.contains(&secret));
        assert!(!preview[0]
            .project_hint
            .as_deref()
            .unwrap()
            .contains(&secret));
        assert!(preview[0].title.contains("[redacted:anthropic]"));
        assert!(preview[0]
            .project_hint
            .as_deref()
            .unwrap()
            .contains("[redacted:anthropic]"));
    }
}

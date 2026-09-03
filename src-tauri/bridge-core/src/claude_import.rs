//! Claude Code source adapter for the shared external-import pipeline.
//!
//! Discovery is deliberately filesystem-only and requires explicit roots.
//! Documented Markdown/configuration surfaces are stable; auto memory and
//! transcripts are parsed only when the caller supplies the named schema gate.

use crate::external_import::{
    canonicalize_approved_path, content_hash, deterministic_identity, sanitize_import_payload,
    source_now, validate_schema_version, CandidateKind, DiscoveredArtifact, DiscoveryRequest,
    DiscoveryResult, DiscoverySelection, ExternalHarnessImporter, FixtureFormat, FixtureManifest,
    ImportCandidate, ImportDiagnostic, NormalizedImportCandidate, SchemaGate, SourceClassification,
    Stability, ValidationResult,
};
use crate::BridgeError;
use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const PROVIDER: &str = "claude_code";
pub const ADAPTER_VERSION: &str = "1";
pub const AUTO_MEMORY_GATE: &str = "claude-auto-memory-v1";
pub const TRANSCRIPT_GATE: &str = "claude-jsonl-v1";
pub const SETTINGS_GATE: &str = "claude-settings-v1";
const MAX_SOURCE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_DISCOVERY_DEPTH: usize = 20;

#[derive(Debug, Default, Clone, Copy)]
pub struct ClaudeCodeImporter;

impl ExternalHarnessImporter for ClaudeCodeImporter {
    fn discover(&self, request: &DiscoveryRequest) -> Result<DiscoveryResult, BridgeError> {
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
            discover_root(root, &approved_roots, &mut artifacts, &mut diagnostics)?;
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
        Ok(DiscoveryResult {
            discovery_id: deterministic_identity(
                PROVIDER,
                &approved_roots
                    .iter()
                    .map(|path| path.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("\n"),
                None,
                &request
                    .format_versions
                    .iter()
                    .map(|(key, value)| format!("{key}={value}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                &CandidateKind::Unsupported,
            ),
            provider: PROVIDER.into(),
            approved_roots: approved_roots
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect(),
            source_version: request.source_version.clone(),
            format_versions: request.format_versions.clone(),
            artifacts,
            diagnostics,
            discovered_at: Utc::now().to_rfc3339(),
        })
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
        for artifact in discovery
            .artifacts
            .iter()
            .filter(|artifact| selected.contains(artifact.artifact_id.as_str()))
        {
            let source_path = canonicalize_approved_path(
                &approved_roots,
                Path::new(&artifact.canonical_source_ref),
            )?;
            enforce_size(&source_path)?;
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
            match artifact.kind {
                CandidateKind::Conversation => {
                    candidates.push(preview_transcript(&source_path, artifact, discovery)?)
                }
                CandidateKind::Memory => candidates.push(preview_markdown(
                    &source_path,
                    artifact,
                    discovery,
                    CandidateKind::Memory,
                )?),
                CandidateKind::Instruction
                | CandidateKind::Rule
                | CandidateKind::Command
                | CandidateKind::Skill
                | CandidateKind::Agent => candidates.push(preview_markdown(
                    &source_path,
                    artifact,
                    discovery,
                    artifact.kind.clone(),
                )?),
                CandidateKind::Prompt => {
                    candidates.extend(preview_settings(&source_path, artifact, discovery)?)
                }
                CandidateKind::McpServer => {
                    candidates.extend(preview_mcp(&source_path, artifact, discovery)?)
                }
                _ => {}
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

fn discover_root(
    root: &Path,
    approved_roots: &[PathBuf],
    artifacts: &mut Vec<DiscoveredArtifact>,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> Result<(), BridgeError> {
    let is_claude_home = root.file_name().and_then(|name| name.to_str()) == Some(".claude");
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
            |path| path.extension().and_then(|part| part.to_str()) == Some("jsonl"),
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
    let mut title = None;
    for (index, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let record: Value = serde_json::from_str(line).map_err(|_| {
            BridgeError::Invalid(format!(
                "Claude transcript line {} is malformed; nothing was imported",
                index + 1
            ))
        })?;
        let object = record.as_object().ok_or_else(|| {
            BridgeError::Invalid(format!(
                "Claude transcript line {} is not an object",
                index + 1
            ))
        })?;
        let record_type = object.get("type").and_then(Value::as_str).ok_or_else(|| {
            BridgeError::Invalid(format!(
                "Claude transcript line {} has no supported record type",
                index + 1
            ))
        })?;
        if !matches!(record_type, "user" | "assistant") {
            return Err(BridgeError::Invalid(format!(
                "Claude transcript record type '{record_type}' is not allowlisted; nothing was imported"
            )));
        }
        let source_id = object.get("uuid").and_then(Value::as_str).ok_or_else(|| {
            BridgeError::Invalid("Claude transcript message is missing uuid".into())
        })?;
        if !seen_ids.insert(source_id.to_string()) {
            return Err(BridgeError::Invalid(
                "Claude transcript contains a duplicate source message id".into(),
            ));
        }
        let timestamp = object
            .get("timestamp")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                BridgeError::Invalid("Claude transcript message is missing timestamp".into())
            })?;
        DateTime::parse_from_rfc3339(timestamp).map_err(|_| {
            BridgeError::Invalid("Claude transcript contains an invalid timestamp".into())
        })?;
        project_hint =
            project_hint.or_else(|| object.get("cwd").and_then(Value::as_str).map(str::to_owned));
        let message = object
            .get("message")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                BridgeError::Invalid("Claude transcript message payload is missing".into())
            })?;
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or(record_type);
        if role != record_type {
            return Err(BridgeError::Invalid(
                "Claude transcript record and message roles disagree".into(),
            ));
        }
        let (text, tool_metadata) = parse_message_content(message.get("content"), record_type)?;
        if title.is_none() && role == "user" && !text.trim().is_empty() {
            title = Some(
                text.lines()
                    .next()
                    .unwrap_or("Imported Claude conversation")
                    .chars()
                    .take(80)
                    .collect::<String>(),
            );
        }
        messages.push(json!({
            "sourceId": source_id,
            "role": role,
            "kind": if role == "user" { "user.message" } else { "assistant.message" },
            "text": text,
            "toolMetadata": tool_metadata,
            "timestamp": timestamp,
            "sequence": messages.len() + 1
        }));
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
        "transcriptParserVersion": TRANSCRIPT_GATE
    });
    let mut result = candidate(
        path,
        artifact,
        discovery,
        CandidateKind::Conversation,
        payload,
        source_native_id.as_deref(),
    )?;
    result.title = title.unwrap_or_else(|| "Imported Claude conversation".into());
    result.created_at = started_at;
    result.updated_at = ended_at;
    result.project_hint = project_hint;
    Ok(result)
}

fn parse_message_content(
    content: Option<&Value>,
    role: &str,
) -> Result<(String, Vec<Value>), BridgeError> {
    match content {
        Some(Value::String(text)) => Ok((text.clone(), Vec::new())),
        Some(Value::Array(blocks)) => {
            let mut text = Vec::new();
            let mut tools = Vec::new();
            for block in blocks {
                let object = block.as_object().ok_or_else(|| {
                    BridgeError::Invalid("Claude transcript content block is not an object".into())
                })?;
                match object.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        let value =
                            object.get("text").and_then(Value::as_str).ok_or_else(|| {
                                BridgeError::Invalid("Claude text block has no text".into())
                            })?;
                        text.push(value.to_string());
                    }
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
                    Some(other) => {
                        return Err(BridgeError::Invalid(format!(
                            "Claude transcript content block '{other}' is not allowlisted"
                        )))
                    }
                    None => {
                        return Err(BridgeError::Invalid(
                            "Claude transcript content block has no type".into(),
                        ))
                    }
                }
            }
            Ok((text.join("\n\n"), tools))
        }
        _ => Err(BridgeError::Invalid(
            "Claude transcript message content is unsupported".into(),
        )),
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
    let source_native_id = native_suffix
        .map(str::to_owned)
        .or_else(|| Some(artifact.source_label.clone()));
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
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[2]["sequence"], 3);
        assert_eq!(messages[1]["sourceId"], "msg-assistant-1");
        assert_eq!(preview[0].project_hint.as_deref(), Some("/safe/project"));
        assert_eq!(
            preview[0].source.schema_version.as_deref(),
            Some(TRANSCRIPT_GATE)
        );
    }

    #[test]
    fn rejects_malformed_unknown_and_duplicate_jsonl_records() {
        for fixture in [
            include_str!("../../../testing/fixtures/import/claude/transcripts/malformed.jsonl"),
            include_str!("../../../testing/fixtures/import/claude/transcripts/unsupported.jsonl"),
            include_str!("../../../testing/fixtures/import/claude/transcripts/duplicate-id.jsonl"),
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
            assert!(ClaudeCodeImporter
                .preview(
                    &discovery,
                    &DiscoverySelection {
                        artifact_ids: vec![transcript.artifact_id.clone()]
                    }
                )
                .is_err());
        }
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
        let mut request = request(&claude);
        request
            .format_versions
            .insert(TRANSCRIPT_GATE.into(), "future-private-v9".into());
        let discovery = ClaudeCodeImporter.discover(&request).unwrap();
        let transcript = discovery
            .artifacts
            .iter()
            .find(|item| item.kind == CandidateKind::Conversation)
            .unwrap();
        let error = ClaudeCodeImporter
            .preview(
                &discovery,
                &DiscoverySelection {
                    artifact_ids: vec![transcript.artifact_id.clone()],
                },
            )
            .unwrap_err();
        assert!(error.to_string().contains("not allowlisted"));
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
}

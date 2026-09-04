//! Provider-neutral, local-only import primitives.
//!
//! Adapters may discover and normalize source-owned data. This module owns the
//! invariants that must not drift between Claude Code, Codex, OpenCode, and
//! Cursor: approved paths, schema gates, deterministic identity, secret
//! exclusion, durable provenance, and the atomic destination schema.

use crate::{secret_interception, BridgeError};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub const IMPORTER_NAMESPACE: &str = "bridge.external-import/v1";
pub const IMPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    Conversation,
    Message,
    Attachment,
    Memory,
    ProjectHint,
    Instruction,
    Rule,
    Prompt,
    Command,
    Skill,
    Agent,
    Hook,
    McpServer,
    Plugin,
    Unsupported,
}

impl CandidateKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::Message => "message",
            Self::Attachment => "attachment",
            Self::Memory => "memory",
            Self::ProjectHint => "project_hint",
            Self::Instruction => "instruction",
            Self::Rule => "rule",
            Self::Prompt => "prompt",
            Self::Command => "command",
            Self::Skill => "skill",
            Self::Agent => "agent",
            Self::Hook => "hook",
            Self::McpServer => "mcp_server",
            Self::Plugin => "plugin",
            Self::Unsupported => "unsupported",
        }
    }

    pub fn is_setup(&self) -> bool {
        matches!(
            self,
            Self::Instruction
                | Self::Rule
                | Self::Prompt
                | Self::Command
                | Self::Skill
                | Self::Agent
                | Self::Hook
                | Self::McpServer
                | Self::Plugin
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Stability {
    Stable,
    VersionGated,
    Experimental,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceClassification {
    Documented,
    VersionGatedPrivate,
    UnsupportedPrivate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportSource {
    pub provider: String,
    pub adapter_version: String,
    pub source_version: Option<String>,
    pub schema_version: Option<String>,
    pub canonical_source_ref: String,
    pub source_path_fingerprint: String,
    pub discovered_at: String,
    #[serde(default)]
    pub source_metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportDiagnostic {
    pub code: String,
    pub severity: String,
    pub classification: SourceClassification,
    pub message: String,
    pub recovery: Option<String>,
    /// Safe relative/category reference only. Never rejected raw content.
    pub source_label: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RedactionSummary {
    pub structured_fields_excluded: u32,
    pub text_values_redacted: u32,
    #[serde(default)]
    pub categories: Vec<String>,
    pub safely_representable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportCandidate {
    pub candidate_id: String,
    pub source: ImportSource,
    pub source_native_id: Option<String>,
    pub kind: CandidateKind,
    pub title: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub project_hint: Option<String>,
    pub content_hash: String,
    pub stability: Stability,
    /// Basis points, so the persisted/wire value is deterministic.
    pub confidence_bps: u16,
    pub selected_by_default: bool,
    pub redaction_summary: RedactionSummary,
    #[serde(default)]
    pub diagnostics: Vec<ImportDiagnostic>,
    pub normalized_payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredArtifact {
    pub artifact_id: String,
    pub canonical_source_ref: String,
    pub source_label: String,
    pub kind: CandidateKind,
    pub classification: SourceClassification,
    pub stability: Stability,
    pub estimated_bytes: u64,
    pub modified_at: Option<String>,
    pub required_schema_gate: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryRequest {
    pub provider: String,
    pub approved_roots: Vec<String>,
    pub selected_export: Option<String>,
    pub source_version: Option<String>,
    pub schema_version: Option<String>,
    #[serde(default)]
    pub format_versions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryResult {
    pub discovery_id: String,
    pub provider: String,
    pub approved_roots: Vec<String>,
    pub source_version: Option<String>,
    #[serde(default)]
    pub format_versions: BTreeMap<String, String>,
    pub artifacts: Vec<DiscoveredArtifact>,
    #[serde(default)]
    pub diagnostics: Vec<ImportDiagnostic>,
    pub discovered_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverySelection {
    pub artifact_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    Skip,
    ImportAsNewHistoricalRevision,
    KeepExisting,
    ImportAlongside,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SetupActivationPolicy {
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportPlan {
    pub selected_candidate_ids: Vec<String>,
    pub conflict_policy: ConflictPolicy,
    pub setup_activation_policy: SetupActivationPolicy,
    pub memory_scope: Option<String>,
    pub dry_run: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SchemaGate {
    pub format: String,
    pub allowed_versions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ValidationResult {
    pub accepted: bool,
    #[serde(default)]
    pub diagnostics: Vec<ImportDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedImportCandidate {
    pub candidate: ImportCandidate,
    pub deterministic_bridge_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportCandidateStatus {
    Imported,
    SkippedDuplicate,
    ChangedSource,
    Conflicted,
    Rejected,
    Unsupported,
    DryRun,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportCandidateResult {
    pub candidate_id: String,
    pub status: ImportCandidateStatus,
    #[serde(default)]
    pub created_bridge_ids: Vec<String>,
    pub revision_of: Option<String>,
    #[serde(default)]
    pub diagnostics: Vec<ImportDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportCommit {
    pub import_id: String,
    pub candidate_results: Vec<ImportCandidateResult>,
    pub created_bridge_ids: Vec<String>,
    pub imported: u32,
    pub skipped: u32,
    pub changed: u32,
    pub conflicted: u32,
    pub rejected: u32,
    pub unsupported: u32,
    pub rollback_state: String,
    #[serde(default)]
    pub diagnostics: Vec<ImportDiagnostic>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FixtureFormat {
    pub source: String,
    pub classification: SourceClassification,
    pub versions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FixtureManifest {
    pub provider: String,
    pub adapter_version: String,
    pub formats: Vec<FixtureFormat>,
}

pub trait ExternalHarnessImporter {
    fn discover(&self, request: &DiscoveryRequest) -> Result<DiscoveryResult, BridgeError>;
    fn preview(
        &self,
        discovery: &DiscoveryResult,
        selection: &DiscoverySelection,
    ) -> Result<Vec<ImportCandidate>, BridgeError>;
    fn validate(&self, candidate: &ImportCandidate, schema_gate: &SchemaGate) -> ValidationResult;
    fn normalize(
        &self,
        candidate: ImportCandidate,
    ) -> Result<NormalizedImportCandidate, BridgeError>;
    fn fixture_manifest(&self) -> FixtureManifest;
}

/// Resolve an existing source while proving its final canonical path is under
/// one of the user-approved canonical roots. This rejects `..` and symlink
/// escapes with the same check.
pub fn canonicalize_approved_path(
    approved_roots: &[PathBuf],
    source: &Path,
) -> Result<PathBuf, BridgeError> {
    let canonical_source = source
        .canonicalize()
        .map_err(|error| BridgeError::Invalid(format!("Import source is unavailable: {error}")))?;
    let approved = approved_roots.iter().any(|root| {
        root.canonicalize()
            .is_ok_and(|canonical_root| canonical_source.starts_with(canonical_root))
    });
    if !approved {
        return Err(BridgeError::Invalid(
            "Import source is outside the user-approved locations".into(),
        ));
    }
    Ok(canonical_source)
}

pub fn path_fingerprint(path: &Path) -> String {
    sha256_hex(path.to_string_lossy().as_bytes())
}

pub fn content_hash(value: &Value) -> String {
    let canonical = canonical_json(value);
    sha256_hex(canonical.to_string().as_bytes())
}

pub fn deterministic_identity(
    provider: &str,
    canonical_source_ref: &str,
    source_native_id: Option<&str>,
    source_content_hash: &str,
    kind: &CandidateKind,
) -> String {
    let mut hasher = Sha256::new();
    for part in [
        IMPORTER_NAMESPACE,
        provider,
        canonical_source_ref,
        source_native_id.unwrap_or(""),
        source_content_hash,
        kind.as_str(),
    ] {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    format!("imp_{}", hex_digest(hasher.finalize()))
}

pub fn validate_schema_version(gate: &SchemaGate, actual: Option<&str>) -> ValidationResult {
    let accepted = actual.is_some_and(|version| gate.allowed_versions.iter().any(|v| v == version));
    if accepted {
        return ValidationResult {
            accepted: true,
            diagnostics: Vec::new(),
        };
    }
    ValidationResult {
        accepted: false,
        diagnostics: vec![ImportDiagnostic {
            code: "unsupported_schema_version".into(),
            severity: "error".into(),
            classification: SourceClassification::VersionGatedPrivate,
            message: format!(
                "{} format version is not allowlisted; Bridge did not parse it",
                gate.format
            ),
            recovery: Some("Use a supported export or manual import instead".into()),
            source_label: None,
        }],
    }
}

/// Structured exclusion runs before text scanning. Keys that can transport
/// authentication or executable environment values are removed whole; all
/// remaining strings pass through the high-confidence credential scanner.
pub fn sanitize_import_payload(payload: &Value) -> (Value, RedactionSummary) {
    let mut summary = RedactionSummary {
        safely_representable: true,
        ..RedactionSummary::default()
    };
    let sanitized = sanitize_value(payload, &mut summary);
    summary.categories.sort();
    summary.categories.dedup();
    (sanitized, summary)
}

fn sanitize_value(value: &Value, summary: &mut RedactionSummary) -> Value {
    match value {
        Value::Object(object) => {
            let mut safe = Map::new();
            for (key, value) in object {
                if excluded_structured_key(key) {
                    summary.structured_fields_excluded += 1;
                    summary.categories.push("structured_auth".into());
                    continue;
                }
                safe.insert(key.clone(), sanitize_value(value, summary));
            }
            Value::Object(safe)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| sanitize_value(item, summary))
                .collect(),
        ),
        Value::String(text) => {
            let turn = secret_interception::sanitize(text);
            if turn.interceptions.is_empty() {
                return Value::String(text.clone());
            }
            summary.text_values_redacted += turn.interceptions.len() as u32;
            let mut redacted = turn.text;
            for item in turn.interceptions {
                redacted = redacted.replace(
                    &format!("[secret:{}]", item.reference),
                    &format!("[redacted:{}]", item.detector),
                );
                summary.categories.push(item.detector);
            }
            Value::String(redacted)
        }
        other => other.clone(),
    }
}

fn excluded_structured_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    matches!(
        normalized.as_str(),
        "authorization"
            | "auth"
            | "apikey"
            | "apisecret"
            | "apitoken"
            | "token"
            | "accesstoken"
            | "refreshtoken"
            | "sessiontoken"
            | "oauthtoken"
            | "oauthstate"
            | "secret"
            | "clientsecret"
            | "cookie"
            | "cookies"
            | "password"
            | "privatekey"
            | "sshkey"
            | "credential"
            | "credentials"
            | "headers"
            | "env"
            | "environment"
            | "connectionstring"
            | "xapikey"
    )
}

fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut entries: Vec<_> = object.iter().collect();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key.clone(), canonical_json(value)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical_json).collect()),
        other => other.clone(),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_digest(hasher.finalize())
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn commit_import(
    db: &Connection,
    candidates: &[NormalizedImportCandidate],
    plan: &ImportPlan,
) -> Result<ImportCommit, BridgeError> {
    commit_import_inner(db, candidates, plan, None)
}

fn commit_import_inner(
    db: &Connection,
    candidates: &[NormalizedImportCandidate],
    plan: &ImportPlan,
    fail_after_candidate_write: Option<usize>,
) -> Result<ImportCommit, BridgeError> {
    let selected = validate_selected_candidates(candidates, plan)?;
    let import_id = format!("import_{}", Uuid::new_v4().simple());
    let created_at = Utc::now().to_rfc3339();
    let transaction = db.unchecked_transaction()?;

    if plan.dry_run {
        let candidate_results = selected
            .iter()
            .map(|candidate| classify_candidate(&transaction, candidate, plan, true))
            .collect::<Result<Vec<_>, _>>()?;
        let mut commit = summarize_commit(import_id, candidate_results, "dry_run", created_at);
        commit.diagnostics.push(ImportDiagnostic {
            code: "dry_run".into(),
            severity: "info".into(),
            classification: SourceClassification::Documented,
            message: "Dry run completed; Bridge wrote no import records".into(),
            recovery: None,
            source_label: None,
        });
        return Ok(commit);
    }

    transaction.execute(
        "INSERT INTO import_commits(id,importer_namespace,provider,plan,outcome,rollback_state,created_at)
         VALUES(?1,?2,?3,?4,'{}','pending',?5)",
        params![
            import_id,
            IMPORTER_NAMESPACE,
            selected
                .first()
                .map(|candidate| candidate.candidate.source.provider.as_str())
                .unwrap_or("unknown"),
            serde_json::to_string(plan).map_err(|error| BridgeError::Invalid(error.to_string()))?,
            created_at,
        ],
    )?;

    let mut candidate_results = Vec::with_capacity(selected.len());
    let mut completed_writes = 0_usize;
    for candidate in selected {
        let classified = classify_candidate(&transaction, candidate, plan, false)?;
        if classified.status != ImportCandidateStatus::Imported {
            persist_candidate_diagnostics(
                &transaction,
                &import_id,
                &candidate.candidate.candidate_id,
                &classified.diagnostics,
                &created_at,
            )?;
            candidate_results.push(classified);
            continue;
        }

        let revision_of = classified.revision_of.clone();
        let created_bridge_ids = write_candidate(
            &transaction,
            &import_id,
            candidate,
            plan,
            revision_of.as_deref(),
            &created_at,
        )?;
        completed_writes += 1;
        if fail_after_candidate_write == Some(completed_writes) {
            return Err(BridgeError::Invalid(format!(
                "injected import failure after candidate write {completed_writes}"
            )));
        }
        candidate_results.push(ImportCandidateResult {
            candidate_id: candidate.candidate.candidate_id.clone(),
            status: ImportCandidateStatus::Imported,
            created_bridge_ids,
            revision_of,
            diagnostics: candidate.candidate.diagnostics.clone(),
        });
    }

    let commit = summarize_commit(
        import_id.clone(),
        candidate_results,
        "committed",
        created_at.clone(),
    );
    transaction.execute(
        "UPDATE import_commits SET outcome=?2,rollback_state='committed' WHERE id=?1",
        params![
            import_id,
            serde_json::to_string(&commit)
                .map_err(|error| BridgeError::Invalid(error.to_string()))?
        ],
    )?;
    transaction.commit()?;
    Ok(commit)
}

fn validate_selected_candidates<'a>(
    candidates: &'a [NormalizedImportCandidate],
    plan: &ImportPlan,
) -> Result<Vec<&'a NormalizedImportCandidate>, BridgeError> {
    if plan.selected_candidate_ids.is_empty() {
        return Err(BridgeError::Invalid(
            "Select at least one import candidate".into(),
        ));
    }
    if plan.setup_activation_policy != SetupActivationPolicy::Disabled {
        return Err(BridgeError::Invalid(
            "Imported setup must remain disabled".into(),
        ));
    }
    let mut by_id = HashMap::new();
    for candidate in candidates {
        if by_id
            .insert(candidate.candidate.candidate_id.as_str(), candidate)
            .is_some()
        {
            return Err(BridgeError::Invalid(
                "Import preview contains duplicate candidate ids".into(),
            ));
        }
    }
    let mut selected_ids = HashSet::new();
    let mut selected = Vec::with_capacity(plan.selected_candidate_ids.len());
    for candidate_id in &plan.selected_candidate_ids {
        if !selected_ids.insert(candidate_id.as_str()) {
            return Err(BridgeError::Invalid(
                "Import plan selects the same candidate more than once".into(),
            ));
        }
        let candidate = by_id.get(candidate_id.as_str()).copied().ok_or_else(|| {
            BridgeError::Invalid(format!(
                "Import plan references unknown candidate '{candidate_id}'"
            ))
        })?;
        validate_candidate_integrity(candidate)?;
        selected.push(candidate);
    }
    if selected
        .iter()
        .any(|candidate| candidate.candidate.kind == CandidateKind::Memory)
    {
        let scope = plan.memory_scope.as_deref().ok_or_else(|| {
            BridgeError::Invalid("Imported memories require an explicit Bridge scope".into())
        })?;
        crate::memory_ledger::parse_scope_key(scope)?;
    }
    Ok(selected)
}

fn validate_candidate_integrity(normalized: &NormalizedImportCandidate) -> Result<(), BridgeError> {
    let candidate = &normalized.candidate;
    if candidate.source.provider.trim().is_empty()
        || candidate.source.adapter_version.trim().is_empty()
        || candidate.source.canonical_source_ref.trim().is_empty()
    {
        return Err(BridgeError::Invalid(
            "Import candidate provenance is incomplete".into(),
        ));
    }
    if candidate.confidence_bps > 10_000 || !candidate.redaction_summary.safely_representable {
        return Err(BridgeError::Invalid(
            "Import candidate is not safely representable".into(),
        ));
    }
    if candidate.kind == CandidateKind::Unsupported {
        return Err(BridgeError::Invalid(
            "Unsupported data is diagnostic-only and cannot be selected".into(),
        ));
    }
    let (resanitized, _) = sanitize_import_payload(&candidate.normalized_payload);
    if resanitized != candidate.normalized_payload
        || content_hash(&candidate.normalized_payload) != candidate.content_hash
    {
        return Err(BridgeError::Invalid(
            "Import candidate changed after privacy scanning; preview again".into(),
        ));
    }
    let title_value = Value::String(candidate.title.clone());
    if sanitize_import_payload(&title_value).0 != title_value {
        return Err(BridgeError::Invalid(
            "Import candidate title contains excluded credential material".into(),
        ));
    }
    let expected_id = deterministic_identity(
        &candidate.source.provider,
        &candidate.source.canonical_source_ref,
        candidate.source_native_id.as_deref(),
        &candidate.content_hash,
        &candidate.kind,
    );
    if candidate.candidate_id != expected_id || normalized.deterministic_bridge_id != expected_id {
        return Err(BridgeError::Invalid(
            "Import candidate identity does not match its normalized content".into(),
        ));
    }
    Ok(())
}

fn classify_candidate(
    db: &Connection,
    normalized: &NormalizedImportCandidate,
    plan: &ImportPlan,
    dry_run: bool,
) -> Result<ImportCandidateResult, BridgeError> {
    let candidate = &normalized.candidate;
    let source_native_key = source_native_key(candidate);
    let exact: Option<String> = db
        .query_row(
            "SELECT bridge_id FROM import_ledger
             WHERE importer_namespace=?1 AND provider=?2 AND source_path_fingerprint=?3
               AND source_native_key=?4 AND source_content_hash=?5 AND candidate_kind=?6",
            params![
                IMPORTER_NAMESPACE,
                candidate.source.provider,
                candidate.source.source_path_fingerprint,
                source_native_key,
                candidate.content_hash,
                candidate.kind.as_str(),
            ],
            |row| row.get(0),
        )
        .optional()?;
    if exact.is_some() {
        return Ok(result_with_status(
            candidate,
            ImportCandidateStatus::SkippedDuplicate,
            None,
            "unchanged_reimport",
            "Unchanged source was already imported; no records were written",
        ));
    }

    let prior: Option<String> = db
        .query_row(
            "SELECT bridge_id FROM import_ledger
             WHERE importer_namespace=?1 AND provider=?2 AND source_path_fingerprint=?3
               AND source_native_key=?4 AND candidate_kind=?5
             ORDER BY created_at DESC LIMIT 1",
            params![
                IMPORTER_NAMESPACE,
                candidate.source.provider,
                candidate.source.source_path_fingerprint,
                source_native_key,
                candidate.kind.as_str(),
            ],
            |row| row.get(0),
        )
        .optional()?;
    if prior.is_some() && plan.conflict_policy != ConflictPolicy::ImportAsNewHistoricalRevision {
        return Ok(result_with_status(
            candidate,
            ImportCandidateStatus::ChangedSource,
            prior,
            "changed_source",
            "Source content changed; choose a new historical revision or skip it",
        ));
    }

    if destination_conflicts(db, candidate, plan)? {
        return Ok(result_with_status(
            candidate,
            ImportCandidateStatus::Conflicted,
            prior,
            "destination_conflict",
            "An existing Bridge record may conflict; choose keep existing, import alongside, or skip",
        ));
    }
    Ok(ImportCandidateResult {
        candidate_id: candidate.candidate_id.clone(),
        status: if dry_run {
            ImportCandidateStatus::DryRun
        } else {
            ImportCandidateStatus::Imported
        },
        created_bridge_ids: Vec::new(),
        revision_of: prior,
        diagnostics: candidate.diagnostics.clone(),
    })
}

fn destination_conflicts(
    db: &Connection,
    candidate: &ImportCandidate,
    plan: &ImportPlan,
) -> Result<bool, BridgeError> {
    if plan.conflict_policy == ConflictPolicy::ImportAlongside {
        return Ok(false);
    }
    if candidate.kind == CandidateKind::Memory {
        let scope = plan.memory_scope.as_deref().ok_or_else(|| {
            BridgeError::Invalid("Imported memories require an explicit Bridge scope".into())
        })?;
        let kind = imported_memory_kind()?;
        return Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM memory_records WHERE scope_key=?1 AND kind=?2 AND status='active')",
            params![scope, kind],
            |row| row.get(0),
        )?);
    }
    if candidate.kind.is_setup() {
        return Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM import_setup_candidates WHERE candidate_kind=?1 AND title=?2)",
            params![candidate.kind.as_str(), candidate.title],
            |row| row.get(0),
        )?);
    }
    Ok(false)
}

fn result_with_status(
    candidate: &ImportCandidate,
    status: ImportCandidateStatus,
    revision_of: Option<String>,
    code: &str,
    message: &str,
) -> ImportCandidateResult {
    ImportCandidateResult {
        candidate_id: candidate.candidate_id.clone(),
        status,
        created_bridge_ids: Vec::new(),
        revision_of,
        diagnostics: vec![ImportDiagnostic {
            code: code.into(),
            severity: "warning".into(),
            classification: if matches!(candidate.stability, Stability::VersionGated) {
                SourceClassification::VersionGatedPrivate
            } else {
                SourceClassification::Documented
            },
            message: message.into(),
            recovery: None,
            source_label: None,
        }],
    }
}

fn write_candidate(
    transaction: &Transaction<'_>,
    import_id: &str,
    normalized: &NormalizedImportCandidate,
    plan: &ImportPlan,
    revision_of: Option<&str>,
    imported_at: &str,
) -> Result<Vec<String>, BridgeError> {
    let candidate = &normalized.candidate;
    let mut created = match candidate.kind {
        CandidateKind::Conversation => {
            write_conversation(transaction, import_id, normalized, imported_at)?
        }
        CandidateKind::Memory => {
            vec![write_memory(transaction, normalized, plan, imported_at)?]
        }
        CandidateKind::ProjectHint => {
            vec![write_project_hint(transaction, normalized, imported_at)?]
        }
        ref kind if kind.is_setup() => {
            vec![write_setup_candidate(
                transaction,
                normalized,
                plan,
                imported_at,
            )?]
        }
        CandidateKind::Message | CandidateKind::Attachment => {
            return Err(BridgeError::Invalid(
                "Messages and attachments must be nested in a conversation candidate".into(),
            ))
        }
        CandidateKind::Unsupported => {
            return Err(BridgeError::Invalid(
                "Unsupported data cannot be committed".into(),
            ))
        }
        _ => {
            return Err(BridgeError::Invalid(
                "Candidate kind has no import destination".into(),
            ))
        }
    };
    let destination_kind = if candidate.kind == CandidateKind::Conversation {
        "session"
    } else if candidate.kind == CandidateKind::Memory {
        "memory"
    } else if candidate.kind == CandidateKind::ProjectHint {
        "project_hint"
    } else {
        "setup"
    };
    insert_provenance(
        transaction,
        &normalized.deterministic_bridge_id,
        destination_kind,
        candidate,
        import_id,
        imported_at,
        None,
        None,
    )?;
    transaction.execute(
        "INSERT INTO import_ledger(importer_namespace,provider,source_path_fingerprint,source_native_key,
            source_content_hash,candidate_kind,import_id,bridge_id,revision_of,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            IMPORTER_NAMESPACE,
            candidate.source.provider,
            candidate.source.source_path_fingerprint,
            source_native_key(candidate),
            candidate.content_hash,
            candidate.kind.as_str(),
            import_id,
            normalized.deterministic_bridge_id,
            revision_of,
            imported_at,
        ],
    )?;
    persist_candidate_diagnostics(
        transaction,
        import_id,
        &candidate.candidate_id,
        &candidate.diagnostics,
        imported_at,
    )?;
    if !created.contains(&normalized.deterministic_bridge_id) {
        created.insert(0, normalized.deterministic_bridge_id.clone());
    }
    Ok(created)
}

fn write_conversation(
    transaction: &Transaction<'_>,
    import_id: &str,
    normalized: &NormalizedImportCandidate,
    imported_at: &str,
) -> Result<Vec<String>, BridgeError> {
    let candidate = &normalized.candidate;
    let messages = candidate
        .normalized_payload
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| BridgeError::Invalid("Conversation candidate has no messages".into()))?;
    if messages.is_empty() {
        return Err(BridgeError::Invalid(
            "Conversation candidate has no messages".into(),
        ));
    }
    transaction.execute(
        "INSERT INTO sessions(id,workspace_id,harness,label,status,started_at,ended_at,metric_source,
            provider_session_id,active_turn_id,title,title_source,kind,cwd)
         VALUES(?1,NULL,'claude','Claude Code Import','stopped',?2,?3,'imported',NULL,NULL,?4,'imported','imported',NULL)",
        params![
            normalized.deterministic_bridge_id,
            candidate.created_at,
            candidate.updated_at,
            candidate.title,
        ],
    )?;
    let mut created = vec![normalized.deterministic_bridge_id.clone()];
    let mut parent: Option<String> = None;
    for (index, message) in messages.iter().enumerate() {
        let source_id = message
            .get("sourceId")
            .and_then(Value::as_str)
            .ok_or_else(|| BridgeError::Invalid("Imported message has no source id".into()))?;
        let kind = message
            .get("kind")
            .and_then(Value::as_str)
            .filter(|kind| matches!(*kind, "user.message" | "assistant.message"))
            .ok_or_else(|| BridgeError::Invalid("Imported message kind is unsupported".into()))?;
        let timestamp = message
            .get("timestamp")
            .and_then(Value::as_str)
            .ok_or_else(|| BridgeError::Invalid("Imported message has no timestamp".into()))?;
        DateTime::parse_from_rfc3339(timestamp)
            .map_err(|_| BridgeError::Invalid("Imported message timestamp is invalid".into()))?;
        let message_hash = content_hash(message);
        let revision_scoped_source_id = format!("{}:{source_id}", candidate.content_hash);
        let entry_id = deterministic_identity(
            &candidate.source.provider,
            &candidate.source.canonical_source_ref,
            Some(&revision_scoped_source_id),
            &message_hash,
            &CandidateKind::Message,
        );
        let mut payload = message.as_object().cloned().ok_or_else(|| {
            BridgeError::Invalid("Imported message payload must be an object".into())
        })?;
        payload.insert("imported".into(), Value::Bool(true));
        payload.insert(
            "sourcePathFingerprint".into(),
            Value::String(candidate.source.source_path_fingerprint.clone()),
        );
        payload.insert(
            crate::session_forest::TYPED_SCHEMA_MARKER.into(),
            Value::from(crate::session_forest::TYPED_SCHEMA_VERSION),
        );
        crate::session_forest::EntryKind::from_storage(kind)
            .validate_payload(&Value::Object(payload.clone()))
            .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        transaction.execute(
            "INSERT INTO session_entries(id,session_id,parent_entry_id,sequence,semantic_schema_version,kind,
                payload,provider_event_id,context_visibility,token_estimate,created_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'visible',?9,?10)",
            params![
                entry_id,
                normalized.deterministic_bridge_id,
                parent,
                (index + 1) as i64,
                crate::model::SEMANTIC_EVENT_SCHEMA_VERSION,
                kind,
                Value::Object(payload).to_string(),
                source_id,
                message
                    .get("text")
                    .and_then(Value::as_str)
                    .map(|text| (text.chars().count() as i64 + 3) / 4),
                timestamp,
            ],
        )?;
        insert_provenance(
            transaction,
            &entry_id,
            "session_entry",
            candidate,
            import_id,
            imported_at,
            Some(source_id),
            Some(&message_hash),
        )?;
        parent = Some(entry_id.clone());
        created.push(entry_id);
    }
    transaction.execute(
        "INSERT INTO session_heads(session_id,active_entry_id,native_provider_session_id,restoration_mode,
            resume_eligibility,latest_checkpoint_entry_id,updated_at)
         VALUES(?1,?2,NULL,'fresh','fresh',NULL,?3)",
        params![normalized.deterministic_bridge_id, parent, imported_at],
    )?;
    if let Some(project_hint) = candidate.project_hint.as_deref() {
        let hint_hash = sha256_hex(project_hint.as_bytes());
        // Scoped by content_hash, like `revision_scoped_source_id` above, so
        // importing a changed-history revision of the same conversation gets
        // its own hint row instead of colliding on the ledger's primary key.
        let revision_scoped_hint = format!("{}:{project_hint}", candidate.content_hash);
        let hint_id = deterministic_identity(
            &candidate.source.provider,
            &candidate.source.canonical_source_ref,
            Some(&revision_scoped_hint),
            &hint_hash,
            &CandidateKind::ProjectHint,
        );
        transaction.execute(
            "INSERT INTO import_project_hints(id,source_hint,matched_workspace_id,match_state,created_at)
             VALUES(?1,?2,NULL,'unconfirmed',?3)",
            params![hint_id, project_hint, imported_at],
        )?;
        insert_provenance(
            transaction,
            &hint_id,
            "project_hint",
            candidate,
            import_id,
            imported_at,
            Some(project_hint),
            Some(&hint_hash),
        )?;
        created.push(hint_id);
    }
    Ok(created)
}

fn write_memory(
    transaction: &Transaction<'_>,
    normalized: &NormalizedImportCandidate,
    plan: &ImportPlan,
    imported_at: &str,
) -> Result<String, BridgeError> {
    let candidate = &normalized.candidate;
    let scope = crate::memory_ledger::parse_scope_key(
        plan.memory_scope
            .as_deref()
            .ok_or_else(|| BridgeError::Invalid("Imported memory needs a scope".into()))?,
    )?;
    let raw_body = candidate
        .normalized_payload
        .get("body")
        .and_then(Value::as_str)
        .ok_or_else(|| BridgeError::Invalid("Imported memory body is empty".into()))?;
    let body = crate::memory_ledger::require_body(raw_body)?;
    let kind = imported_memory_kind()?;
    crate::memory_consolidation::enforce_scope_budget(transaction, &scope)?;
    let conflict_group = existing_memory_conflict(transaction, &scope, kind)?.map(|_| {
        deterministic_identity(
            "bridge",
            &scope,
            Some(kind),
            "memory-conflict",
            &CandidateKind::Memory,
        )
    });
    transaction.execute(
        "INSERT INTO memory_records(id,scope_key,kind,body,provenance,status,source_session_id,
            confidence_bps,rationale,valid_from,valid_to,expires_at,conflict_group,created_at,updated_at)
         VALUES(?1,?2,?3,?4,'claude_code:auto_memory','active',NULL,?5,
            'Imported from Claude Code after explicit scope selection',?6,NULL,NULL,?7,?6,?6)",
        params![
            normalized.deterministic_bridge_id,
            scope,
            kind,
            body,
            i64::from(candidate.confidence_bps),
            imported_at,
            conflict_group,
        ],
    )?;
    Ok(normalized.deterministic_bridge_id.clone())
}

fn write_setup_candidate(
    transaction: &Transaction<'_>,
    normalized: &NormalizedImportCandidate,
    plan: &ImportPlan,
    imported_at: &str,
) -> Result<String, BridgeError> {
    if plan.setup_activation_policy != SetupActivationPolicy::Disabled {
        return Err(BridgeError::Invalid(
            "Imported setup must remain disabled".into(),
        ));
    }
    let candidate = &normalized.candidate;
    let conflict: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM import_setup_candidates WHERE candidate_kind=?1 AND title=?2)",
        params![candidate.kind.as_str(), candidate.title],
        |row| row.get(0),
    )?;
    let conflict_group = conflict.then(|| {
        deterministic_identity(
            "bridge",
            candidate.kind.as_str(),
            Some(&candidate.title),
            "setup-conflict",
            &candidate.kind,
        )
    });
    transaction.execute(
        "INSERT INTO import_setup_candidates(id,candidate_kind,title,payload,activation_state,conflict_group,created_at)
         VALUES(?1,?2,?3,?4,'disabled',?5,?6)",
        params![
            normalized.deterministic_bridge_id,
            candidate.kind.as_str(),
            candidate.title,
            candidate.normalized_payload.to_string(),
            conflict_group,
            imported_at,
        ],
    )?;
    Ok(normalized.deterministic_bridge_id.clone())
}

fn write_project_hint(
    transaction: &Transaction<'_>,
    normalized: &NormalizedImportCandidate,
    imported_at: &str,
) -> Result<String, BridgeError> {
    let hint = normalized
        .candidate
        .normalized_payload
        .get("path")
        .and_then(Value::as_str)
        .or(normalized.candidate.project_hint.as_deref())
        .ok_or_else(|| BridgeError::Invalid("Project hint candidate has no path".into()))?;
    transaction.execute(
        "INSERT INTO import_project_hints(id,source_hint,matched_workspace_id,match_state,created_at)
         VALUES(?1,?2,NULL,'unconfirmed',?3)",
        params![normalized.deterministic_bridge_id, hint, imported_at],
    )?;
    Ok(normalized.deterministic_bridge_id.clone())
}

#[allow(clippy::too_many_arguments)]
fn insert_provenance(
    transaction: &Transaction<'_>,
    bridge_id: &str,
    destination_kind: &str,
    candidate: &ImportCandidate,
    import_id: &str,
    imported_at: &str,
    source_native_id: Option<&str>,
    source_content_hash: Option<&str>,
) -> Result<(), BridgeError> {
    transaction.execute(
        "INSERT INTO imported_record_provenance(bridge_id,destination_kind,provider,adapter_version,
            source_version,schema_version,canonical_source_ref,source_path_fingerprint,source_native_id,
            source_content_hash,imported_at,redaction_summary,stability,confidence_bps,import_id)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
        params![
            bridge_id,
            destination_kind,
            candidate.source.provider,
            candidate.source.adapter_version,
            candidate.source.source_version,
            candidate.source.schema_version,
            candidate.source.canonical_source_ref,
            candidate.source.source_path_fingerprint,
            source_native_id.or(candidate.source_native_id.as_deref()),
            source_content_hash.unwrap_or(&candidate.content_hash),
            imported_at,
            serde_json::to_string(&candidate.redaction_summary)
                .map_err(|error| BridgeError::Invalid(error.to_string()))?,
            serde_json::to_value(&candidate.stability)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_else(|| "unavailable".into()),
            i64::from(candidate.confidence_bps),
            import_id,
        ],
    )?;
    Ok(())
}

fn persist_candidate_diagnostics(
    transaction: &Transaction<'_>,
    import_id: &str,
    candidate_id: &str,
    diagnostics: &[ImportDiagnostic],
    created_at: &str,
) -> Result<(), BridgeError> {
    for diagnostic in diagnostics {
        transaction.execute(
            "INSERT INTO import_diagnostics(import_id,candidate_id,code,severity,classification,safe_summary,recovery,created_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                import_id,
                candidate_id,
                diagnostic.code,
                diagnostic.severity,
                serde_json::to_value(&diagnostic.classification)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .unwrap_or_else(|| "unsupported_private".into()),
                diagnostic.message,
                diagnostic.recovery,
                created_at,
            ],
        )?;
    }
    Ok(())
}

fn existing_memory_conflict(
    db: &Connection,
    scope: &str,
    kind: &str,
) -> Result<Option<String>, BridgeError> {
    Ok(db
        .query_row(
            "SELECT id FROM memory_records WHERE scope_key=?1 AND kind=?2 AND status='active'
             ORDER BY updated_at DESC LIMIT 1",
            params![scope, kind],
            |row| row.get(0),
        )
        .optional()?)
}

/// Claude's own memory taxonomy (an `index` doc vs a per-topic note) has no
/// counterpart in `memory_ledger`'s closed kind set, so every imported memory
/// lands as a `fact` — the ledger's most neutral kind. This keeps imported
/// records inside `memory_ledger::parse_kind`'s allowlist (so they round-trip
/// through the editor) and makes them a real target for conflict detection
/// against Bridge-authored memories, instead of a kind value no other writer
/// ever produces.
fn imported_memory_kind() -> Result<&'static str, BridgeError> {
    crate::memory_ledger::parse_kind(Some("fact"))
}

fn source_native_key(candidate: &ImportCandidate) -> &str {
    candidate
        .source_native_id
        .as_deref()
        .unwrap_or(&candidate.source.canonical_source_ref)
}

fn summarize_commit(
    import_id: String,
    candidate_results: Vec<ImportCandidateResult>,
    rollback_state: &str,
    created_at: String,
) -> ImportCommit {
    let created_bridge_ids = candidate_results
        .iter()
        .flat_map(|result| result.created_bridge_ids.iter().cloned())
        .collect();
    let count = |status| {
        candidate_results
            .iter()
            .filter(|result| result.status == status)
            .count() as u32
    };
    ImportCommit {
        import_id,
        imported: count(ImportCandidateStatus::Imported),
        skipped: count(ImportCandidateStatus::SkippedDuplicate),
        changed: count(ImportCandidateStatus::ChangedSource),
        conflicted: count(ImportCandidateStatus::Conflicted),
        rejected: count(ImportCandidateStatus::Rejected),
        unsupported: count(ImportCandidateStatus::Unsupported),
        candidate_results,
        created_bridge_ids,
        rollback_state: rollback_state.into(),
        diagnostics: Vec::new(),
        created_at,
    }
}

pub(crate) fn install_import_foundation(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS import_commits (
            id TEXT PRIMARY KEY,
            importer_namespace TEXT NOT NULL,
            provider TEXT NOT NULL,
            plan TEXT NOT NULL,
            outcome TEXT NOT NULL,
            rollback_state TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS import_ledger (
            importer_namespace TEXT NOT NULL,
            provider TEXT NOT NULL,
            source_path_fingerprint TEXT NOT NULL,
            source_native_key TEXT NOT NULL,
            source_content_hash TEXT NOT NULL,
            candidate_kind TEXT NOT NULL,
            import_id TEXT NOT NULL REFERENCES import_commits(id),
            bridge_id TEXT NOT NULL,
            revision_of TEXT,
            created_at TEXT NOT NULL,
            PRIMARY KEY(importer_namespace,provider,source_path_fingerprint,source_native_key,source_content_hash,candidate_kind)
        );
        CREATE INDEX IF NOT EXISTS idx_import_ledger_source_revision
            ON import_ledger(importer_namespace,provider,source_path_fingerprint,source_native_key,candidate_kind,created_at);
        CREATE TABLE IF NOT EXISTS imported_record_provenance (
            bridge_id TEXT PRIMARY KEY,
            destination_kind TEXT NOT NULL,
            provider TEXT NOT NULL,
            adapter_version TEXT NOT NULL,
            source_version TEXT,
            schema_version TEXT,
            canonical_source_ref TEXT NOT NULL,
            source_path_fingerprint TEXT NOT NULL,
            source_native_id TEXT,
            source_content_hash TEXT NOT NULL,
            imported_at TEXT NOT NULL,
            redaction_summary TEXT NOT NULL,
            stability TEXT NOT NULL,
            confidence_bps INTEGER NOT NULL CHECK(confidence_bps BETWEEN 0 AND 10000),
            import_id TEXT NOT NULL REFERENCES import_commits(id)
        );
        CREATE TABLE IF NOT EXISTS import_setup_candidates (
            id TEXT PRIMARY KEY,
            candidate_kind TEXT NOT NULL,
            title TEXT NOT NULL,
            payload TEXT NOT NULL,
            activation_state TEXT NOT NULL DEFAULT 'disabled' CHECK(activation_state = 'disabled'),
            conflict_group TEXT,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS import_project_hints (
            id TEXT PRIMARY KEY,
            source_hint TEXT NOT NULL,
            matched_workspace_id TEXT REFERENCES workspaces(id),
            match_state TEXT NOT NULL DEFAULT 'unconfirmed' CHECK(match_state IN ('unconfirmed','confirmed','declined')),
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS import_diagnostics (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            import_id TEXT NOT NULL REFERENCES import_commits(id),
            candidate_id TEXT,
            code TEXT NOT NULL,
            severity TEXT NOT NULL,
            classification TEXT NOT NULL,
            safe_summary TEXT NOT NULL,
            recovery TEXT,
            created_at TEXT NOT NULL
        );
        CREATE TRIGGER IF NOT EXISTS imported_provenance_immutable_update
        BEFORE UPDATE ON imported_record_provenance BEGIN
            SELECT RAISE(ABORT, 'import provenance is immutable');
        END;
        CREATE TRIGGER IF NOT EXISTS imported_provenance_immutable_delete
        BEFORE DELETE ON imported_record_provenance BEGIN
            SELECT RAISE(ABORT, 'import provenance is immutable');
        END;
        CREATE TRIGGER IF NOT EXISTS imported_session_entries_immutable_update
        BEFORE UPDATE ON session_entries
        WHEN EXISTS (
            SELECT 1 FROM imported_record_provenance p
            WHERE p.bridge_id = OLD.session_id AND p.destination_kind = 'session'
        ) BEGIN
            SELECT RAISE(ABORT, 'imported session history is immutable');
        END;
        CREATE TRIGGER IF NOT EXISTS imported_session_entries_immutable_delete
        BEFORE DELETE ON session_entries
        WHEN EXISTS (
            SELECT 1 FROM imported_record_provenance p
            WHERE p.bridge_id = OLD.session_id AND p.destination_kind = 'session'
        ) BEGIN
            SELECT RAISE(ABORT, 'imported session history is immutable');
        END;
        CREATE TRIGGER IF NOT EXISTS imported_session_entries_immutable_insert
        BEFORE INSERT ON session_entries
        WHEN EXISTS (
            SELECT 1 FROM imported_record_provenance p
            WHERE p.bridge_id = NEW.session_id AND p.destination_kind = 'session'
        ) BEGIN
            SELECT RAISE(ABORT, 'imported session history is immutable');
        END;",
    )?;
    Ok(())
}

pub fn source_now(
    provider: &str,
    adapter_version: &str,
    canonical_source_ref: &Path,
    source_version: Option<String>,
    schema_version: Option<String>,
) -> ImportSource {
    ImportSource {
        provider: provider.into(),
        adapter_version: adapter_version.into(),
        source_version,
        schema_version,
        canonical_source_ref: canonical_source_ref.to_string_lossy().into_owned(),
        source_path_fingerprint: path_fingerprint(canonical_source_ref),
        discovered_at: Utc::now().to_rfc3339(),
        source_metadata: Value::Object(Map::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_identity_is_stable_and_namespaced() {
        let first = deterministic_identity(
            "claude_code",
            "/approved/CLAUDE.md",
            Some("rule-one"),
            "abc",
            &CandidateKind::Rule,
        );
        let same = deterministic_identity(
            "claude_code",
            "/approved/CLAUDE.md",
            Some("rule-one"),
            "abc",
            &CandidateKind::Rule,
        );
        assert_eq!(first, same);
        assert_ne!(
            first,
            deterministic_identity(
                "codex",
                "/approved/CLAUDE.md",
                Some("rule-one"),
                "abc",
                &CandidateKind::Rule,
            )
        );
        assert_ne!(
            first,
            deterministic_identity(
                "claude_code",
                "/approved/CLAUDE.md",
                Some("rule-one"),
                "changed",
                &CandidateKind::Rule,
            )
        );
    }

    #[test]
    fn secret_filter_excludes_structured_auth_and_redacts_text() {
        let anthropic_secret =
            ["sk", "ant", "this-is-a-secret-value-with-enough-entropy"].join("-");
        let payload = serde_json::json!({
            "authorization": format!("Bearer {anthropic_secret}"),
            "env": {"SAFE": "still excluded", "TOKEN": anthropic_secret},
            "body": format!("please use {anthropic_secret}"),
            "nested": {"name": "safe"}
        });
        let (safe, summary) = sanitize_import_payload(&payload);
        let encoded = safe.to_string();
        assert!(!encoded.contains(&anthropic_secret));
        assert!(safe.get("authorization").is_none());
        assert!(safe.get("env").is_none());
        assert_eq!(safe["body"], "please use [redacted:anthropic]");
        assert_eq!(summary.structured_fields_excluded, 2);
        assert_eq!(summary.text_values_redacted, 1);
        assert!(summary.safely_representable);
    }

    #[test]
    fn unknown_schema_versions_fail_closed() {
        let gate = SchemaGate {
            format: "Claude Code transcript JSONL".into(),
            allowed_versions: vec!["claude-jsonl-v1".into()],
        };
        assert!(validate_schema_version(&gate, Some("claude-jsonl-v1")).accepted);
        let rejected = validate_schema_version(&gate, Some("future-v9"));
        assert!(!rejected.accepted);
        assert_eq!(rejected.diagnostics[0].code, "unsupported_schema_version");
        assert!(rejected.diagnostics[0]
            .recovery
            .as_deref()
            .unwrap()
            .contains("manual import"));
    }

    #[test]
    fn migration_installs_shared_import_tables_and_immutability() {
        let db = crate::store::open(Path::new(":memory:")).unwrap();
        for table in [
            "import_commits",
            "import_ledger",
            "imported_record_provenance",
            "import_setup_candidates",
            "import_project_hints",
            "import_diagnostics",
        ] {
            let exists: bool = db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(exists, "missing {table}");
        }
        db.execute("INSERT INTO import_commits(id,importer_namespace,provider,plan,outcome,rollback_state,created_at) VALUES('i',?1,'claude_code','{}','{}','committed','now')", [IMPORTER_NAMESPACE]).unwrap();
        db.execute("INSERT INTO imported_record_provenance(bridge_id,destination_kind,provider,adapter_version,canonical_source_ref,source_path_fingerprint,source_content_hash,imported_at,redaction_summary,stability,confidence_bps,import_id) VALUES('b','setup','claude_code','1','/approved','fp','hash','now','{}','stable',10000,'i')", []).unwrap();
        assert!(db
            .execute(
                "UPDATE imported_record_provenance SET confidence_bps=0 WHERE bridge_id='b'",
                [],
            )
            .is_err());
    }

    fn normalized(
        kind: CandidateKind,
        native_id: &str,
        title: &str,
        payload: Value,
    ) -> NormalizedImportCandidate {
        let (payload, redaction_summary) = sanitize_import_payload(&payload);
        let hash = content_hash(&payload);
        let source = source_now(
            "claude_code",
            "1",
            Path::new("/approved/claude-source"),
            Some("2.1.59".into()),
            Some("fixture-v1".into()),
        );
        let id = deterministic_identity(
            &source.provider,
            &source.canonical_source_ref,
            Some(native_id),
            &hash,
            &kind,
        );
        NormalizedImportCandidate {
            deterministic_bridge_id: id.clone(),
            candidate: ImportCandidate {
                candidate_id: id,
                source,
                source_native_id: Some(native_id.into()),
                kind,
                title: title.into(),
                created_at: Some("2026-08-01T10:00:00Z".into()),
                updated_at: Some("2026-08-01T10:00:02Z".into()),
                project_hint: None,
                content_hash: hash,
                stability: Stability::VersionGated,
                confidence_bps: 8_000,
                selected_by_default: false,
                redaction_summary,
                diagnostics: Vec::new(),
                normalized_payload: payload,
            },
        }
    }

    fn conversation(native_id: &str, second_message: &str) -> NormalizedImportCandidate {
        normalized(
            CandidateKind::Conversation,
            native_id,
            "Imported conversation",
            serde_json::json!({
                "messages": [
                    {
                        "sourceId": "message-1",
                        "role": "user",
                        "kind": "user.message",
                        "text": "Build the import foundation",
                        "toolMetadata": [],
                        "timestamp": "2026-08-01T10:00:00Z",
                        "sequence": 1
                    },
                    {
                        "sourceId": "message-2",
                        "role": "assistant",
                        "kind": "assistant.message",
                        "text": second_message,
                        "toolMetadata": [],
                        "timestamp": "2026-08-01T10:00:02Z",
                        "sequence": 2
                    }
                ],
                "historical": true,
                "resumable": false,
                "sourceBadge": "Imported from Claude Code"
            }),
        )
    }

    fn plan(
        candidates: &[NormalizedImportCandidate],
        conflict_policy: ConflictPolicy,
    ) -> ImportPlan {
        ImportPlan {
            selected_candidate_ids: candidates
                .iter()
                .map(|candidate| candidate.candidate.candidate_id.clone())
                .collect(),
            conflict_policy,
            setup_activation_policy: SetupActivationPolicy::Disabled,
            memory_scope: Some("account:local".into()),
            dry_run: false,
            created_at: "2026-08-01T11:00:00Z".into(),
        }
    }

    fn row_count(db: &Connection, table: &str) -> i64 {
        db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    #[test]
    fn atomic_commit_persists_history_memory_setup_provenance_and_ledger() {
        let db = crate::store::open(Path::new(":memory:")).unwrap();
        let mut history = conversation("session-1", "The shared foundation is ready.");
        history.candidate.project_hint = Some("/source/project".into());
        let memory = normalized(
            CandidateKind::Memory,
            "memory-1",
            "MEMORY.md",
            serde_json::json!({
                "body": "Use the shared import transaction.",
                "memoryType": "index",
                "proposedScope": null,
                "activationState": "requires_scope_decision"
            }),
        );
        let setup = normalized(
            CandidateKind::Rule,
            "rule-1",
            "testing.md",
            serde_json::json!({
                "body": "Always run the import tests.",
                "activationState": "disabled",
                "sourceKind": "rule"
            }),
        );
        let candidates = vec![history, memory, setup];
        let committed = commit_import(
            &db,
            &candidates,
            &plan(&candidates, ConflictPolicy::ImportAlongside),
        )
        .unwrap();

        assert_eq!(committed.imported, 3);
        assert_eq!(committed.rollback_state, "committed");
        assert_eq!(row_count(&db, "sessions"), 1);
        assert_eq!(row_count(&db, "session_entries"), 2);
        assert_eq!(row_count(&db, "memory_records"), 1);
        assert_eq!(row_count(&db, "import_setup_candidates"), 1);
        assert_eq!(row_count(&db, "import_project_hints"), 1);
        assert_eq!(row_count(&db, "import_ledger"), 3);
        assert_eq!(row_count(&db, "imported_record_provenance"), 6);
        let session: (String, String, Option<String>, String) = db
            .query_row(
                "SELECT kind,status,provider_session_id,metric_source FROM sessions",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            session,
            ("imported".into(), "stopped".into(), None, "imported".into())
        );
        let head: (String, String, Option<String>) = db
            .query_row(
                "SELECT restoration_mode,resume_eligibility,native_provider_session_id FROM session_heads",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(head, ("fresh".into(), "fresh".into(), None));
        assert!(db
            .execute("UPDATE session_entries SET kind='changed'", [])
            .is_err());
        assert!(db.execute("DELETE FROM session_entries", []).is_err());
        let session_id: String = db
            .query_row("SELECT id FROM sessions LIMIT 1", [], |row| row.get(0))
            .unwrap();
        assert!(db
            .execute(
                "INSERT INTO session_entries(id,session_id,sequence,kind,created_at)
                 VALUES('injected',?1,99,'user.message','now')",
                params![session_id],
            )
            .is_err());
    }

    #[test]
    fn unchanged_reimport_is_a_noop_and_changed_source_requires_choice() {
        let db = crate::store::open(Path::new(":memory:")).unwrap();
        let initial = vec![conversation("session-1", "First result")];
        commit_import(&db, &initial, &plan(&initial, ConflictPolicy::Skip)).unwrap();
        let duplicate =
            commit_import(&db, &initial, &plan(&initial, ConflictPolicy::Skip)).unwrap();
        assert_eq!(duplicate.skipped, 1);
        assert!(duplicate.created_bridge_ids.is_empty());
        assert_eq!(row_count(&db, "sessions"), 1);

        let changed = vec![conversation("session-1", "Changed result")];
        let review = commit_import(&db, &changed, &plan(&changed, ConflictPolicy::Skip)).unwrap();
        assert_eq!(review.changed, 1);
        assert_eq!(row_count(&db, "sessions"), 1);
        let revision = commit_import(
            &db,
            &changed,
            &plan(&changed, ConflictPolicy::ImportAsNewHistoricalRevision),
        )
        .unwrap();
        assert_eq!(revision.imported, 1);
        assert!(revision.candidate_results[0].revision_of.is_some());
        assert_eq!(row_count(&db, "sessions"), 2);
    }

    #[test]
    fn memory_conflicts_require_keep_skip_or_import_alongside() {
        let db = crate::store::open(Path::new(":memory:")).unwrap();
        db.execute(
            "INSERT INTO memory_records(id,scope_key,kind,body,provenance,status,valid_from,created_at,updated_at)
             VALUES('existing','account:local','fact','Existing memory','user_explicit','active','now','now','now')",
            [],
        )
        .unwrap();
        let memory = vec![normalized(
            CandidateKind::Memory,
            "memory-1",
            "MEMORY.md",
            serde_json::json!({"body":"Imported memory","memoryType":"index"}),
        )];
        let skipped =
            commit_import(&db, &memory, &plan(&memory, ConflictPolicy::KeepExisting)).unwrap();
        assert_eq!(skipped.conflicted, 1);
        assert_eq!(row_count(&db, "memory_records"), 1);

        let alongside = commit_import(
            &db,
            &memory,
            &plan(&memory, ConflictPolicy::ImportAlongside),
        )
        .unwrap();
        assert_eq!(alongside.imported, 1);
        assert_eq!(row_count(&db, "memory_records"), 2);
        let conflict_group: Option<String> = db
            .query_row(
                "SELECT conflict_group FROM memory_records WHERE id<>'existing'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(conflict_group.is_some());
    }

    #[test]
    fn failure_after_each_destination_candidate_rolls_back_every_import_write() {
        let candidates = vec![
            conversation("session-1", "Done"),
            normalized(
                CandidateKind::Memory,
                "memory-1",
                "MEMORY.md",
                serde_json::json!({"body":"Imported memory","memoryType":"index"}),
            ),
            normalized(
                CandidateKind::Rule,
                "rule-1",
                "testing.md",
                serde_json::json!({"body":"Run tests","activationState":"disabled"}),
            ),
        ];
        for failure_point in 1..=3 {
            let db = crate::store::open(Path::new(":memory:")).unwrap();
            let result = commit_import_inner(
                &db,
                &candidates,
                &plan(&candidates, ConflictPolicy::ImportAlongside),
                Some(failure_point),
            );
            assert!(result.is_err(), "failure point {failure_point} committed");
            for table in [
                "sessions",
                "session_entries",
                "memory_records",
                "import_setup_candidates",
                "import_project_hints",
                "imported_record_provenance",
                "import_ledger",
                "import_commits",
                "import_diagnostics",
            ] {
                assert_eq!(
                    row_count(&db, table),
                    0,
                    "{table} survived point {failure_point}"
                );
            }
        }
    }

    #[test]
    fn dry_run_writes_nothing() {
        let db = crate::store::open(Path::new(":memory:")).unwrap();
        let candidates = vec![conversation("session-1", "Done")];
        let mut dry_run = plan(&candidates, ConflictPolicy::Skip);
        dry_run.dry_run = true;
        let result = commit_import(&db, &candidates, &dry_run).unwrap();
        assert_eq!(result.rollback_state, "dry_run");
        assert_eq!(
            result.candidate_results[0].status,
            ImportCandidateStatus::DryRun
        );
        assert_eq!(row_count(&db, "sessions"), 0);
        assert_eq!(row_count(&db, "import_commits"), 0);
    }

    #[test]
    fn imported_sessions_cannot_start_as_provider_sessions() {
        let scratch = tempfile::tempdir().unwrap();
        let core = std::sync::Arc::new(crate::runtime::BridgeCore::for_tests(scratch.path()));
        let candidates = vec![conversation("session-1", "Done")];
        {
            let db = core.db.lock().unwrap();
            commit_import(&db, &candidates, &plan(&candidates, ConflictPolicy::Skip)).unwrap();
        }
        let error = crate::api::start_chat(&core, candidates[0].deterministic_bridge_id.clone())
            .unwrap_err();
        assert!(error.to_string().contains("cannot resume"));
    }

    /// The composer's send path never calls `api::start_chat` — it resumes a
    /// stopped session through `live_turn::resume_for_send` ->
    /// `live_turn::start_chat` directly. The gate has to live in that shared
    /// function, or typing into an imported chat silently launches a real
    /// provider session on top of immutable history.
    #[test]
    fn imported_sessions_cannot_resume_through_the_composer_send_path() {
        let scratch = tempfile::tempdir().unwrap();
        let core = std::sync::Arc::new(crate::runtime::BridgeCore::for_tests(scratch.path()));
        let candidates = vec![conversation("session-1", "Done")];
        {
            let db = core.db.lock().unwrap();
            commit_import(&db, &candidates, &plan(&candidates, ConflictPolicy::Skip)).unwrap();
        }
        let error = crate::live_turn::send_turn(
            &core,
            candidates[0].deterministic_bridge_id.clone(),
            "hello".into(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("cannot resume"));
        assert_eq!(row_count(&core.db.lock().unwrap(), "session_entries"), 2);
    }

    /// An imported conversation has no workspace, so `sessions.workspace_id`
    /// is NULL — `session_forest_snapshot` used to read that column as a
    /// non-null `String` and fail every fetch, leaving the imported chat
    /// permanently blank in the UI.
    #[test]
    fn imported_sessions_have_a_fetchable_forest_snapshot() {
        let db = crate::store::open(Path::new(":memory:")).unwrap();
        let candidates = vec![conversation("session-1", "Done")];
        commit_import(&db, &candidates, &plan(&candidates, ConflictPolicy::Skip)).unwrap();
        let snapshot = crate::sessions::session_forest_snapshot(
            &db,
            &candidates[0].deterministic_bridge_id,
        )
        .unwrap();
        assert_eq!(snapshot.entries.len(), 2);
    }
}

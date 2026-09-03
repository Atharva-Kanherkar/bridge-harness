//! Provider-neutral, local-only import primitives.
//!
//! Adapters may discover and normalize source-owned data. This module owns the
//! invariants that must not drift between Claude Code, Codex, OpenCode, and
//! Cursor: approved paths, schema gates, deterministic identity, secret
//! exclusion, durable provenance, and the atomic destination schema.

use crate::{secret_interception, BridgeError};
use chrono::Utc;
use rusqlite::Transaction;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryRequest {
    pub provider: String,
    pub approved_roots: Vec<String>,
    pub selected_export: Option<String>,
    pub source_version: Option<String>,
    pub schema_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryResult {
    pub discovery_id: String,
    pub provider: String,
    pub approved_roots: Vec<String>,
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
            | "accesstoken"
            | "refreshtoken"
            | "oauthtoken"
            | "oauthstate"
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

pub(crate) fn install_import_foundation(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE import_commits (
            id TEXT PRIMARY KEY,
            importer_namespace TEXT NOT NULL,
            provider TEXT NOT NULL,
            plan TEXT NOT NULL,
            outcome TEXT NOT NULL,
            rollback_state TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE import_ledger (
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
        CREATE INDEX idx_import_ledger_source_revision
            ON import_ledger(importer_namespace,provider,source_path_fingerprint,source_native_key,candidate_kind,created_at);
        CREATE TABLE imported_record_provenance (
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
        CREATE TABLE import_setup_candidates (
            id TEXT PRIMARY KEY,
            candidate_kind TEXT NOT NULL,
            title TEXT NOT NULL,
            payload TEXT NOT NULL,
            activation_state TEXT NOT NULL DEFAULT 'disabled' CHECK(activation_state = 'disabled'),
            conflict_group TEXT,
            created_at TEXT NOT NULL
        );
        CREATE TABLE import_project_hints (
            id TEXT PRIMARY KEY,
            source_hint TEXT NOT NULL,
            matched_workspace_id TEXT REFERENCES workspaces(id),
            match_state TEXT NOT NULL DEFAULT 'unconfirmed' CHECK(match_state IN ('unconfirmed','confirmed','declined')),
            created_at TEXT NOT NULL
        );
        CREATE TABLE import_diagnostics (
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
        CREATE TRIGGER imported_provenance_immutable_update
        BEFORE UPDATE ON imported_record_provenance BEGIN
            SELECT RAISE(ABORT, 'import provenance is immutable');
        END;
        CREATE TRIGGER imported_provenance_immutable_delete
        BEFORE DELETE ON imported_record_provenance BEGIN
            SELECT RAISE(ABORT, 'import provenance is immutable');
        END;
        CREATE TRIGGER imported_session_entries_immutable_update
        BEFORE UPDATE ON session_entries
        WHEN EXISTS (
            SELECT 1 FROM imported_record_provenance p
            WHERE p.bridge_id = OLD.session_id AND p.destination_kind = 'session'
        ) BEGIN
            SELECT RAISE(ABORT, 'imported session history is immutable');
        END;
        CREATE TRIGGER imported_session_entries_immutable_delete
        BEFORE DELETE ON session_entries
        WHEN EXISTS (
            SELECT 1 FROM imported_record_provenance p
            WHERE p.bridge_id = OLD.session_id AND p.destination_kind = 'session'
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
}

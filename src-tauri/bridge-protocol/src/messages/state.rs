//! The aggregate application snapshot: `state/get_state`'s result, and the
//! result of every mutation that returns the refreshed state. Mirrors the
//! `bridge_core::model` DTOs field for field; the drift gate in bridge-core's
//! `protocol_mirror` keeps the two in wire-value lockstep.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::common::{JsSafeI64, StoredHarnessId};

/// Mirrors `bridge_core::model::SessionStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Idle,
    Starting,
    Working,
    Waiting,
    Warm,
    Checkpointing,
    Ready,
    Stopped,
    Resuming,
    Restored,
    Failed,
    Completed,
    Cancelled,
}

/// Mirrors `bridge_core::model::CapabilityTier`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CapabilityTier {
    Fast,
    Standard,
    Strong,
}

/// Mirrors `bridge_core::model::RestorationMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RestorationMode {
    Hot,
    Native,
    CheckpointRestored,
    Fresh,
}

/// Mirrors `bridge_core::model::ContinuationFidelity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContinuationFidelity {
    Native,
    ProjectedAtBoundary,
    ProjectedMidTurn,
}

/// Mirrors `bridge_core::model::ResumeEligibility`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResumeEligibility {
    Native,
    CheckpointRestored,
    Fresh,
}

/// Mirrors `bridge_core::model::Project`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
    pub created_at: String,
}

/// Mirrors `bridge_core::model::Workspace`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub id: String,
    pub project_id: Option<String>,
    pub city: Option<String>,
    pub title: String,
    pub branch: Option<String>,
    pub path: Option<String>,
    pub status: SessionStatus,
    pub dirty_files: JsSafeI64,
    pub additions: JsSafeI64,
    pub deletions: JsSafeI64,
    pub created_at: String,
}

/// Mirrors `bridge_core::model::Session`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub workspace_id: Option<String>,
    /// Tolerant on purpose: a session whose agent this build cannot interpret
    /// still reports the id it was stored with. See [`StoredHarnessId`].
    pub harness: StoredHarnessId,
    pub label: String,
    pub status: SessionStatus,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub context_percent: Option<JsSafeI64>,
    pub usage_percent: Option<JsSafeI64>,
    pub metric_source: String,
    pub provider_session_id: Option<String>,
    pub active_turn_id: Option<String>,
    pub model: Option<String>,
    pub requested_tier: Option<CapabilityTier>,
    pub effort: Option<String>,
    pub parent_session_id: Option<String>,
    pub depth: Option<JsSafeI64>,
    pub restoration_mode: RestorationMode,
    pub continuation_fidelity: ContinuationFidelity,
    pub title: Option<String>,
    pub kind: String,
    pub cwd: Option<String>,
}

/// Mirrors `bridge_core::model::BridgeEvent` — the audit/event feed rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BridgeEvent {
    pub id: JsSafeI64,
    pub source: String,
    pub kind: String,
    pub entity_id: String,
    pub body: String,
    pub created_at: String,
}

/// Mirrors `bridge_core::model::BridgeState` — the aggregate snapshot most
/// mutations return so clients render without a follow-up read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BridgeState {
    pub projects: Vec<Project>,
    pub workspaces: Vec<Workspace>,
    pub sessions: Vec<Session>,
    pub events: Vec<BridgeEvent>,
}

/// Mirrors `bridge_core::model::ModelOption`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelOption {
    pub id: String,
    pub label: String,
    pub tier: CapabilityTier,
    #[serde(default = "model_option_default_true")]
    pub available: bool,
    #[serde(default = "model_option_default_true")]
    pub compatible: bool,
    #[serde(default)]
    pub lifecycle: ModelLifecycle,
    #[serde(default)]
    pub source: ModelCatalogSource,
    pub default_for_tier: bool,
}

fn model_option_default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModelLifecycle {
    Stable,
    Preview,
    Deprecated,
    Unknown,
}

impl Default for ModelLifecycle {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModelCatalogSource {
    RuntimeApi,
    LastKnownGood,
    CuratedFallback,
}

impl Default for ModelCatalogSource {
    fn default() -> Self {
        Self::CuratedFallback
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogDiagnostics {
    pub source: ModelCatalogSource,
    pub fetched_at: Option<String>,
    pub expires_at: Option<String>,
    pub stale: bool,
    pub last_error: Option<String>,
}

impl Default for ModelCatalogDiagnostics {
    fn default() -> Self {
        Self {
            source: ModelCatalogSource::CuratedFallback,
            fetched_at: None,
            expires_at: None,
            stale: false,
            last_error: None,
        }
    }
}

/// Mirrors `bridge_core::model::SandboxMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SandboxMode {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

/// Mirrors `bridge_core::model::AuthState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthState {
    SignedIn,
    SignedOut,
    Unknown,
}

/// Mirrors `bridge_core::model::AdapterDescriptor`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdapterDescriptor {
    pub id: String,
    pub label: String,
    pub available: bool,
    pub auth_state: AuthState,
    pub version: Option<String>,
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub sandbox_modes: Vec<SandboxMode>,
    pub unavailable_reason: Option<String>,
    pub models: Vec<ModelOption>,
    pub default_model: Option<String>,
    #[serde(default)]
    pub model_catalog: ModelCatalogDiagnostics,
}

/// One actionable environment warning on the health response. Mirrors
/// `bridge_core::health::HealthWarning`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HealthWarning {
    /// Stable kebab-case id, e.g. `macos-tcc-protected-path`.
    pub id: String,
    pub title: String,
    /// Actionable guidance: the symptom, the cause, and where the fix is.
    pub detail: String,
    /// The offending registered paths; empty when the warning is not about paths.
    pub paths: Vec<String>,
}

/// `health/health`'s result. Mirrors `bridge_core::api::Health`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HealthResult {
    pub ok: bool,
    pub version: String,
    /// Harness id → binary availability.
    pub harnesses: std::collections::BTreeMap<String, bool>,
    pub database: String,
    #[serde(rename = "telemetry_database")]
    pub telemetry_database: String,
    #[serde(rename = "snapshot_directory")]
    pub snapshot_directory: String,
    #[serde(rename = "snapshot_count")]
    pub snapshot_count: u64,
    #[serde(rename = "snapshot_total_bytes")]
    pub snapshot_total_bytes: u64,
    pub adapters: Vec<AdapterDescriptor>,
    /// Actionable environment warnings (today: macOS TCC-protected project
    /// paths and ad-hoc code signing). Defaulted so a document from an older
    /// daemon still parses.
    #[serde(default)]
    pub warnings: Vec<HealthWarning>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::common::round_trip;
    use serde_json::json;

    #[test]
    fn the_state_snapshot_round_trips_with_camel_case_wire_names() {
        let state = BridgeState {
            projects: vec![Project {
                id: "p-1".into(),
                name: "Demo".into(),
                path: "/repos/demo".into(),
                created_at: "now".into(),
            }],
            workspaces: vec![Workspace {
                id: "w-1".into(),
                project_id: Some("p-1".into()),
                city: Some("Kyoto".into()),
                title: "Payments".into(),
                branch: Some("bridge/payments".into()),
                path: Some("/repos/demo".into()),
                status: SessionStatus::Working,
                dirty_files: JsSafeI64::new(2).unwrap(),
                additions: JsSafeI64::new(40).unwrap(),
                deletions: JsSafeI64::new(3).unwrap(),
                created_at: "now".into(),
            }],
            sessions: vec![Session {
                id: "s-1".into(),
                workspace_id: Some("w-1".into()),
                harness: StoredHarnessId::new("codex"),
                label: "Orchestrator".into(),
                status: SessionStatus::Waiting,
                started_at: Some("now".into()),
                ended_at: None,
                context_percent: Some(JsSafeI64::new(41).unwrap()),
                usage_percent: Some(JsSafeI64::new(12).unwrap()),
                metric_source: "reported".into(),
                provider_session_id: Some("prov-1".into()),
                active_turn_id: Some("turn-1".into()),
                model: Some("gpt-5".into()),
                requested_tier: Some(CapabilityTier::Standard),
                effort: Some("high".into()),
                parent_session_id: None,
                depth: Some(JsSafeI64::new(0).unwrap()),
                restoration_mode: RestorationMode::Fresh,
                continuation_fidelity: ContinuationFidelity::Native,
                title: None,
                kind: "orchestrator".into(),
                cwd: Some("/repos/demo".into()),
            }],
            events: vec![BridgeEvent {
                id: JsSafeI64::new(9).unwrap(),
                source: "supervisor".into(),
                kind: "workspace.created".into(),
                entity_id: "w-1".into(),
                body: "Created workspace Payments".into(),
                created_at: "now".into(),
            }],
        };
        let wire = serde_json::to_value(&state).unwrap();
        assert_eq!(wire["workspaces"][0]["projectId"], json!("p-1"));
        assert_eq!(wire["workspaces"][0]["dirtyFiles"], json!(2));
        assert_eq!(wire["sessions"][0]["harness"], json!("codex"));
        assert_eq!(wire["sessions"][0]["status"], json!("waiting"));
        assert_eq!(wire["sessions"][0]["restorationMode"], json!("fresh"));
        assert_eq!(wire["sessions"][0]["continuationFidelity"], json!("native"));
        assert_eq!(wire["events"][0]["entityId"], json!("w-1"));
        assert_eq!(round_trip(&state), state);
    }

    #[test]
    fn snapshot_enums_carry_their_pinned_wire_values() {
        assert_eq!(
            serde_json::to_value(SessionStatus::Checkpointing).unwrap(),
            json!("checkpointing")
        );
        assert_eq!(
            serde_json::to_value(RestorationMode::CheckpointRestored).unwrap(),
            json!("checkpoint_restored")
        );
        assert_eq!(
            serde_json::to_value(ContinuationFidelity::ProjectedMidTurn).unwrap(),
            json!("projected_mid_turn")
        );
        assert_eq!(
            serde_json::to_value(ResumeEligibility::Native).unwrap(),
            json!("native")
        );
        assert!(serde_json::from_value::<SessionStatus>(json!("Working")).is_err());
        assert!(serde_json::from_value::<CapabilityTier>(json!("premium")).is_err());
        assert_eq!(serde_json::to_value(AuthState::SignedIn).unwrap(), json!("signed_in"));
        assert_eq!(serde_json::to_value(AuthState::SignedOut).unwrap(), json!("signed_out"));
        assert_eq!(serde_json::to_value(AuthState::Unknown).unwrap(), json!("unknown"));
    }

    #[test]
    fn health_results_round_trip() {
        let health = HealthResult {
            ok: true,
            version: "0.1.0".into(),
            harnesses: [("claude".to_owned(), true), ("shell".to_owned(), true)].into(),
            database: "/data/bridge.db".into(),
            telemetry_database: "/data/bridge-telemetry.db".into(),
            snapshot_directory: "/data/history-snapshots".into(),
            snapshot_count: 9,
            snapshot_total_bytes: 4_096,
            adapters: vec![AdapterDescriptor {
                id: "codex".into(),
                label: "Codex".into(),
                available: true,
                auth_state: AuthState::SignedIn,
                version: Some("1.0".into()),
                capabilities: vec!["shell".into()],
                sandbox_modes: vec![SandboxMode::ReadOnly, SandboxMode::WorkspaceWrite],
                unavailable_reason: None,
                models: vec![ModelOption {
                    id: "gpt-5".into(),
                    label: "GPT-5".into(),
                    tier: CapabilityTier::Strong,
                    available: true,
                    compatible: true,
                    lifecycle: ModelLifecycle::Stable,
                    source: ModelCatalogSource::CuratedFallback,
                    default_for_tier: true,
                }],
                default_model: Some("gpt-5".into()),
                model_catalog: ModelCatalogDiagnostics {
                    source: ModelCatalogSource::CuratedFallback,
                    fetched_at: None,
                    expires_at: None,
                    stale: false,
                    last_error: None,
                },
            }],
            warnings: vec![HealthWarning {
                id: "macos-tcc-protected-path".into(),
                title: "Project folders sit inside macOS-protected locations".into(),
                detail: "See \u{201c}macOS file access prompts\u{201d} in README.md.".into(),
                paths: vec!["/Users/dev/Documents/app".into()],
            }],
        };
        let wire = serde_json::to_value(&health).unwrap();
        assert_eq!(wire["adapters"][0]["authState"], json!("signed_in"));
        assert_eq!(wire["adapters"][0]["models"][0]["defaultForTier"], json!(true));
        assert_eq!(wire["harnesses"]["claude"], json!(true));
        assert_eq!(wire["telemetry_database"], json!("/data/bridge-telemetry.db"));
        assert_eq!(wire["snapshot_directory"], json!("/data/history-snapshots"));
        assert!(wire.get("telemetryDatabase").is_none());
        assert!(wire.get("snapshotDirectory").is_none());
        assert_eq!(wire["warnings"][0]["id"], json!("macos-tcc-protected-path"));
        assert_eq!(round_trip(&health), health);
    }

    #[test]
    fn a_health_document_from_an_older_daemon_parses_without_warnings() {
        let mut wire = serde_json::to_value(&HealthResult {
            ok: true,
            version: "0.1.0".into(),
            harnesses: Default::default(),
            database: "/data/bridge.db".into(),
            telemetry_database: "/data/bridge-telemetry.db".into(),
            snapshot_directory: "/data/history-snapshots".into(),
            snapshot_count: 0,
            snapshot_total_bytes: 0,
            adapters: vec![],
            warnings: vec![],
        })
        .unwrap();
        wire.as_object_mut().unwrap().remove("warnings");
        let parsed: HealthResult = serde_json::from_value(wire).unwrap();
        assert!(parsed.warnings.is_empty());
    }
}

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum CapabilityTier {
    Fast,
    Standard,
    Strong,
}

impl CapabilityTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Standard => "standard",
            Self::Strong => "strong",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RestorationMode {
    Hot,
    Native,
    CheckpointRestored,
    #[default]
    Fresh,
}

impl RestorationMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hot => "hot",
            Self::Native => "native",
            Self::CheckpointRestored => "checkpoint_restored",
            Self::Fresh => "fresh",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResumeEligibility {
    Native,
    CheckpointRestored,
    #[default]
    Fresh,
}

impl ResumeEligibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::CheckpointRestored => "checkpoint_restored",
            Self::Fresh => "fresh",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Harness {
    Claude,
    Codex,
    Shell,
}

impl Harness {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::Shell => "Shell",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Idle,
    Working,
    Waiting,
    Ready,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub id: String,
    pub project_id: String,
    pub city: String,
    pub title: String,
    pub branch: String,
    pub path: String,
    pub status: SessionStatus,
    pub dirty_files: i64,
    pub additions: i64,
    pub deletions: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelOption {
    pub id: String,
    pub label: String,
    pub tier: CapabilityTier,
    pub default_for_tier: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterDescriptor {
    pub id: String,
    pub label: String,
    pub available: bool,
    pub version: Option<String>,
    pub capabilities: Vec<String>,
    pub unavailable_reason: Option<String>,
    pub models: Vec<ModelOption>,
    pub default_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub workspace_id: String,
    pub harness: Harness,
    pub label: String,
    pub status: SessionStatus,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub context_percent: Option<i64>,
    pub usage_percent: Option<i64>,
    pub metric_source: String,
    pub provider_session_id: Option<String>,
    pub active_turn_id: Option<String>,
    pub model: Option<String>,
    pub requested_tier: Option<CapabilityTier>,
    pub effort: Option<String>,
    pub parent_session_id: Option<String>,
    pub depth: Option<i64>,
    pub restoration_mode: RestorationMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeEvent {
    pub id: i64,
    pub source: String,
    pub kind: String,
    pub entity_id: String,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeState {
    pub projects: Vec<Project>,
    pub workspaces: Vec<Workspace>,
    pub sessions: Vec<Session>,
    pub events: Vec<BridgeEvent>,
    pub agent_events: Vec<AgentEvent>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalChunk {
    pub session_id: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEvent {
    pub id: i64,
    pub session_id: String,
    pub sequence: i64,
    pub protocol_version: i64,
    pub kind: String,
    pub item_id: Option<String>,
    pub role: Option<String>,
    pub status: Option<String>,
    pub title: Option<String>,
    pub text: Option<String>,
    pub data: serde_json::Value,
    pub provider_meta: serde_json::Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionEntry {
    pub id: String,
    pub session_id: String,
    pub parent_entry_id: Option<String>,
    pub sequence: i64,
    pub kind: String,
    pub payload: serde_json::Value,
    pub provider_event_id: Option<String>,
    pub context_visibility: String,
    pub token_estimate: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionHead {
    pub session_id: String,
    pub active_entry_id: Option<String>,
    pub native_provider_session_id: Option<String>,
    pub restoration_mode: RestorationMode,
    pub resume_eligibility: ResumeEligibility,
    pub latest_checkpoint_entry_id: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TaskKnowledge {
    pub id: String,
    pub workspace_id: String,
    pub session_id: Option<String>,
    pub kind: String,
    pub body: String,
    pub source_entry_id: Option<String>,
    pub superseded_by: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkerLease {
    pub session_id: String,
    pub workspace_id: String,
    pub role: String,
    pub capability_tier: String,
    pub task_family: String,
    pub owned_paths: serde_json::Value,
    pub write_mode: String,
    pub lease_status: String,
    pub expires_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkerRuntimeRecord {
    pub session_id: String,
    pub parent_session_id: String,
    pub lifecycle_state: String,
    pub task_family: String,
    pub compatibility_key: String,
    pub result_status: String,
    pub retry_count: i64,
    pub warm_until: Option<String>,
    pub last_result: Option<serde_json::Value>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QueuedWorkerRequest {
    pub id: String,
    pub parent_session_id: String,
    pub workspace_id: String,
    pub turn_id: String,
    pub request: serde_json::Value,
    pub actual_model: String,
    pub queue_status: String,
    pub sequence: i64,
    pub dispatched_session_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsageLedgerRow {
    pub id: i64,
    pub workspace_id: String,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
    pub cache_write_tokens: Option<i64>,
    pub context_percent: Option<i64>,
    pub capability_units: i64,
    pub runtime_ms: Option<i64>,
    pub source: String,
    pub created_at: String,
}

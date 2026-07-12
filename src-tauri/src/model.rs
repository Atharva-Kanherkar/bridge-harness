use serde::{Deserialize, Serialize};

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
    pub fn command(&self) -> (&'static str, Vec<&'static str>) {
        match self {
            Self::Claude => ("claude", vec![]),
            Self::Codex => ("codex", vec![]),
            Self::Shell => ("zsh", vec!["-l"]),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterDescriptor {
    pub id: String,
    pub label: String,
    pub available: bool,
    pub version: Option<String>,
    pub capabilities: Vec<String>,
    pub unavailable_reason: Option<String>,
}

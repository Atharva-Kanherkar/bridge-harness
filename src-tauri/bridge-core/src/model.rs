use bridge_protocol::messages::{HarnessId, HarnessIdError, StoredHarnessId};
use serde::ser::Error as _;
use serde::{Deserialize, Serialize, Serializer};
use std::borrow::Cow;

pub(crate) fn serialize_js_safe_i64<S: Serializer>(
    value: &i64,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    if (-bridge_protocol::MAX_SAFE_INTEGER..=bridge_protocol::MAX_SAFE_INTEGER).contains(value) {
        serializer.serialize_i64(*value)
    } else {
        Err(S::Error::custom(
            "integer exceeds JavaScript's safe integer range",
        ))
    }
}

pub(crate) fn serialize_optional_js_safe_i64<S: Serializer>(
    value: &Option<i64>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        Some(value)
            if !(-bridge_protocol::MAX_SAFE_INTEGER..=bridge_protocol::MAX_SAFE_INTEGER)
                .contains(value) =>
        {
            Err(S::Error::custom(
                "integer exceeds JavaScript's safe integer range",
            ))
        }
        Some(value) => serializer.serialize_some(value),
        None => serializer.serialize_none(),
    }
}

pub(crate) fn serialize_js_safe_usize<S: Serializer>(
    value: &usize,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    if (*value as u64) <= bridge_protocol::MAX_SAFE_INTEGER as u64 {
        serializer.serialize_u64(*value as u64)
    } else {
        Err(S::Error::custom(
            "integer exceeds JavaScript's safe integer range",
        ))
    }
}

pub const SEMANTIC_EVENT_SCHEMA_VERSION: i64 = 2;
pub const MIN_SUPPORTED_SEMANTIC_EVENT_SCHEMA_VERSION: i64 = 1;

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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContinuationFidelity {
    #[default]
    Native,
    ProjectedAtBoundary,
    ProjectedMidTurn,
}

impl ContinuationFidelity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::ProjectedAtBoundary => "projected_at_boundary",
            Self::ProjectedMidTurn => "projected_mid_turn",
        }
    }
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

/// Which harness runs a session.
///
/// Closed over the built-ins on purpose — each has a hand-written adapter and
/// bespoke behaviour, so an exhaustive `match` must go on failing to compile
/// until a new built-in is handled everywhere. The two open arms carry what a
/// closed enum cannot express, and the wire type
/// [`bridge_protocol::messages::HarnessId`] is open for the same reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Harness {
    Claude,
    Codex,
    Cursor,
    Grok,
    OpenCode,
    Shell,
    /// Any other agent — installed from the registry and run through a
    /// generic transport. Its id is validated on construction.
    ///
    /// **Construct only through [`Harness::parse`] or
    /// [`Harness::from_stored`],** which route a built-in name to its own
    /// variant first. `Agent(HarnessId("claude"))` built by hand would
    /// serialize identically to [`Harness::Claude`] while comparing unequal to
    /// it; the constructors cannot produce that.
    Agent(HarnessId),
    /// A stored harness id this build cannot interpret — a row written by a
    /// newer Bridge, or a corrupted one. The raw value is preserved so the
    /// session still lists and replays under its own name.
    ///
    /// Unreachable from the wire: [`Harness::parse`] rejects what
    /// [`Harness::from_stored`] tolerates. It is never a guess about what the
    /// user meant, and it is never runnable — looking it up in the adapter
    /// registry misses, which is the legible failure.
    ///
    /// **Construct only through [`Harness::from_stored`].** Building it
    /// directly with an id that *does* parse — `Harness::Unknown("claude")` —
    /// makes a value that serializes identically to [`Harness::Claude`] while
    /// comparing unequal to it. `from_stored` cannot produce that, and
    /// `a_stored_harness_id_is_idempotent_through_its_canonical_form` pins it.
    Unknown(String),
}

impl Harness {
    /// The canonical id: the wire value, the `sessions.harness` column value,
    /// and the adapter-registry key. One spelling, one place it comes from.
    pub fn id(&self) -> Cow<'static, str> {
        match self {
            Self::Claude => Cow::Borrowed("claude"),
            Self::Codex => Cow::Borrowed("codex"),
            Self::Cursor => Cow::Borrowed("cursor"),
            Self::Grok => Cow::Borrowed("grok"),
            Self::OpenCode => Cow::Borrowed("opencode"),
            Self::Shell => Cow::Borrowed("shell"),
            Self::Agent(agent) => Cow::Owned(agent.as_str().to_owned()),
            Self::Unknown(raw) => Cow::Owned(raw.clone()),
        }
    }

    /// Parse an id supplied by a caller. Strict: exactly what the wire type
    /// accepts, so the daemon and the Tauri host validate identically. An
    /// unrecognized id is an error here, never [`Harness::Unknown`].
    pub fn parse(value: &str) -> Result<Self, HarnessIdError> {
        HarnessId::parse(value).map(Self::from)
    }

    /// Interpret an id read back from storage. Total by necessity: a row that
    /// this build cannot parse still has to load, so it becomes
    /// [`Harness::Unknown`] carrying the raw value.
    ///
    /// This replaced a `_ => Harness::Shell` fallthrough, which turned a
    /// corrupt or forward-dated row into a *runnable shell session*.
    pub fn from_stored(value: &str) -> Self {
        Self::parse(value).unwrap_or_else(|_| Self::Unknown(value.to_owned()))
    }

    pub fn label(&self) -> Cow<'static, str> {
        match self {
            Self::Claude => Cow::Borrowed("Claude"),
            Self::Codex => Cow::Borrowed("Codex"),
            Self::Cursor => Cow::Borrowed("Cursor"),
            Self::Grok => Cow::Borrowed("Grok Build"),
            Self::OpenCode => Cow::Borrowed("OpenCode"),
            Self::Shell => Cow::Borrowed("Shell"),
            // An agent Bridge has no bespoke adapter for has no display name
            // of Bridge's invention; the id the user installed it by is what
            // they are shown.
            Self::Agent(agent) => Cow::Owned(agent.as_str().to_owned()),
            Self::Unknown(raw) => Cow::Owned(raw.clone()),
        }
    }
}

impl From<HarnessId> for Harness {
    /// Total. A built-in name routes to its own variant so the two spellings
    /// can never both exist; everything else is an ordinary agent.
    /// [`Harness::Unknown`] is not reachable through this conversion.
    fn from(id: HarnessId) -> Self {
        match id.as_str() {
            "claude" => Self::Claude,
            "codex" => Self::Codex,
            "cursor" => Self::Cursor,
            "grok" => Self::Grok,
            "opencode" => Self::OpenCode,
            "shell" => Self::Shell,
            _ => Self::Agent(id),
        }
    }
}

impl TryFrom<&Harness> for HarnessId {
    type Error = HarnessIdError;

    /// Fallible in exactly one case: [`Harness::Unknown`] has no valid wire
    /// id. That asymmetry is deliberate — such a session is still *serialized*
    /// under its raw id so its history renders, but it can never be sent back
    /// as a parameter, because nothing can be done with it.
    fn try_from(harness: &Harness) -> Result<Self, Self::Error> {
        HarnessId::parse(&harness.id())
    }
}

impl From<&Harness> for StoredHarnessId {
    /// Total, unlike the [`HarnessId`] conversion. This is the result-side id:
    /// every harness a session can be in has one, including
    /// [`Harness::Unknown`], because a session must remain readable even when
    /// its harness cannot be acted on.
    fn from(harness: &Harness) -> Self {
        StoredHarnessId::new(harness.id().into_owned())
    }
}

impl Serialize for Harness {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.id())
    }
}

impl<'de> Deserialize<'de> for Harness {
    /// Strict, matching the wire type. Storage reads use
    /// [`Harness::from_stored`] instead.
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(&String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
    pub project_id: Option<String>,
    pub city: Option<String>,
    pub title: String,
    pub branch: Option<String>,
    pub path: Option<String>,
    pub status: SessionStatus,
    #[serde(serialize_with = "serialize_js_safe_i64")]
    pub dirty_files: i64,
    #[serde(serialize_with = "serialize_js_safe_i64")]
    pub additions: i64,
    #[serde(serialize_with = "serialize_js_safe_i64")]
    pub deletions: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelOption {
    pub id: String,
    pub label: String,
    pub tier: CapabilityTier,
    /// Selectability is independent from promotion. Discovery can retain an
    /// inaccessible entry for diagnostics without ever routing work to it.
    #[serde(default = "model_option_default_true")]
    pub available: bool,
    /// A runtime may advertise a model whose protocol/tool contract Bridge
    /// cannot satisfy. It remains visible but cannot be selected or promoted.
    #[serde(default = "model_option_default_true")]
    pub compatible: bool,
    #[serde(default)]
    pub lifecycle: ModelLifecycle,
    #[serde(default)]
    pub source: ModelCatalogSource,
    /// Bridge's promoted model for this tier, not merely the provider's
    /// advertised default. Exactly one eligible entry per populated tier wins.
    pub default_for_tier: bool,
}

fn model_option_default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
        Self::curated()
    }
}

impl ModelCatalogDiagnostics {
    pub fn curated() -> Self {
        Self {
            source: ModelCatalogSource::CuratedFallback,
            fetched_at: None,
            expires_at: None,
            stale: false,
            last_error: None,
        }
    }
}

/// Isolation level a worker runs under. The wire values match the sandbox names
/// the router records on each candidate, so a descriptor's declaration and a
/// route decision are directly comparable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxMode {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

impl SandboxMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::WorkspaceWrite => "workspace_write",
            Self::DangerFullAccess => "danger_full_access",
        }
    }

    pub const ALL: [Self; 3] = [Self::ReadOnly, Self::WorkspaceWrite, Self::DangerFullAccess];
}

/// Whether a provider's own credential store holds a usable credential —
/// orthogonal to whether its CLI binary is installed. Derived from a
/// presence/parse check only: Bridge never reads, stores, or logs the
/// credential value itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthState {
    SignedIn,
    SignedOut,
    /// The probe could not determine presence (e.g. an unreadable or
    /// unparsable store). Never guessed as `SignedOut` — a probe error must
    /// not read as "no credential".
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterDescriptor {
    pub id: String,
    pub label: String,
    pub available: bool,
    /// Independent of `available`: a provider whose binary cannot be
    /// resolved still reports its own credential-store probe rather than a
    /// forced `SignedOut`.
    pub auth_state: AuthState,
    pub version: Option<String>,
    pub capabilities: Vec<String>,
    /// Sandbox modes this harness can actually start in, including transport
    /// constraints. A harness that cannot run read-only must not advertise it:
    /// the router uses this to exclude routes that are guaranteed to fail at
    /// adapter startup instead of discovering the failure after spawning.
    #[serde(default)]
    pub sandbox_modes: Vec<SandboxMode>,
    pub unavailable_reason: Option<String>,
    pub models: Vec<ModelOption>,
    pub default_model: Option<String>,
    #[serde(default)]
    pub model_catalog: ModelCatalogDiagnostics,
}

impl AdapterDescriptor {
    /// An empty declaration is treated as "unconstrained" so third-party or
    /// replayed descriptors are never silently excluded.
    pub fn supports_sandbox(&self, mode: SandboxMode) -> bool {
        self.sandbox_modes.is_empty() || self.sandbox_modes.contains(&mode)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub workspace_id: Option<String>,
    pub harness: Harness,
    pub label: String,
    pub status: SessionStatus,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    #[serde(serialize_with = "serialize_optional_js_safe_i64")]
    pub context_percent: Option<i64>,
    #[serde(serialize_with = "serialize_optional_js_safe_i64")]
    pub usage_percent: Option<i64>,
    pub metric_source: String,
    pub provider_session_id: Option<String>,
    pub active_turn_id: Option<String>,
    pub model: Option<String>,
    pub requested_tier: Option<CapabilityTier>,
    pub effort: Option<String>,
    pub parent_session_id: Option<String>,
    #[serde(serialize_with = "serialize_optional_js_safe_i64")]
    pub depth: Option<i64>,
    pub restoration_mode: RestorationMode,
    pub continuation_fidelity: ContinuationFidelity,
    pub title: Option<String>,
    pub kind: String,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeEvent {
    #[serde(serialize_with = "serialize_js_safe_i64")]
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
    #[serde(serialize_with = "serialize_js_safe_i64")]
    pub sequence: i64,
    #[serde(serialize_with = "serialize_js_safe_i64")]
    pub semantic_schema_version: i64,
    pub kind: String,
    pub payload: serde_json::Value,
    pub provider_event_id: Option<String>,
    pub context_visibility: String,
    #[serde(serialize_with = "serialize_optional_js_safe_i64")]
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
    #[serde(serialize_with = "serialize_js_safe_i64")]
    pub retry_count: i64,
    pub warm_until: Option<String>,
    pub worktree_path: Option<String>,
    pub worktree_branch: Option<String>,
    pub last_result: Option<serde_json::Value>,
    pub last_activity_at: Option<String>,
    // Written by their own UPDATE statements, never by the upsert: the upsert
    // races the approval and event paths that maintain them, and a stale DTO
    // must not clobber a fresher observation.
    #[serde(default)]
    pub waiting_since: Option<String>,
    #[serde(default)]
    pub waiting_reason: Option<String>,
    #[serde(default)]
    pub progress_summary: Option<String>,
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
    #[serde(serialize_with = "serialize_js_safe_i64")]
    pub sequence: i64,
    pub dispatched_session_id: Option<String>,
    #[serde(serialize_with = "serialize_js_safe_i64")]
    pub attempt_count: i64,
    pub expires_at: String,
    pub blocked_at: Option<String>,
    pub claimed_at: Option<String>,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OutboxMessage {
    pub id: String,
    pub destination: String,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub idempotency_key: String,
    pub status: String,
    pub attempt_count: i64,
    pub next_attempt_at: String,
    pub last_error: Option<String>,
    pub created_at: String,
    pub delivered_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsageLedgerRow {
    #[serde(serialize_with = "serialize_js_safe_i64")]
    pub id: i64,
    pub workspace_id: String,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    #[serde(serialize_with = "serialize_optional_js_safe_i64")]
    pub input_tokens: Option<i64>,
    #[serde(serialize_with = "serialize_optional_js_safe_i64")]
    pub output_tokens: Option<i64>,
    #[serde(serialize_with = "serialize_optional_js_safe_i64")]
    pub cache_read_tokens: Option<i64>,
    #[serde(serialize_with = "serialize_optional_js_safe_i64")]
    pub cache_write_tokens: Option<i64>,
    #[serde(serialize_with = "serialize_optional_js_safe_i64")]
    pub uncached_input_tokens: Option<i64>,
    #[serde(serialize_with = "serialize_optional_js_safe_i64")]
    pub context_percent: Option<i64>,
    #[serde(serialize_with = "serialize_js_safe_i64")]
    pub capability_units: i64,
    #[serde(serialize_with = "serialize_optional_js_safe_i64")]
    pub runtime_ms: Option<i64>,
    #[serde(serialize_with = "serialize_optional_js_safe_i64")]
    pub cost_microusd: Option<i64>,
    pub cost_source: Option<String>,
    pub stable_prefix_id: Option<String>,
    pub stable_prefix_hash: Option<String>,
    #[serde(serialize_with = "serialize_optional_js_safe_i64")]
    pub prompt_schema_version: Option<i64>,
    #[serde(serialize_with = "serialize_optional_js_safe_i64")]
    pub prefix_token_estimate: Option<i64>,
    pub harness: Option<String>,
    pub model: Option<String>,
    pub role: Option<String>,
    pub task_family: Option<String>,
    pub restoration_mode: Option<String>,
    pub cross_harness_reuse: Option<String>,
    pub source: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PromptCompilationRecord {
    pub id: i64,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub prefix_id: String,
    pub prefix_hash: String,
    pub schema_version: i64,
    pub prefix_bytes: i64,
    pub prefix_token_estimate: i64,
    pub harness: String,
    pub model: Option<String>,
    pub role: String,
    pub task_family: String,
    pub restoration_mode: String,
    pub cross_harness_reuse: String,
    pub created_at: String,
    pub sections_json: Option<String>,
    pub stable_bytes: Option<i64>,
    pub variable_bytes: Option<i64>,
    pub stable_token_estimate: Option<i64>,
    pub variable_token_estimate: Option<i64>,
    pub token_estimate_source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PolicyLimits {
    #[serde(serialize_with = "serialize_js_safe_i64")]
    pub max_workers_per_turn: i64,
    #[serde(serialize_with = "serialize_js_safe_i64")]
    pub max_strong_workers_per_turn: i64,
    #[serde(serialize_with = "serialize_js_safe_i64")]
    pub max_capability_units_per_turn: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionForestSnapshot {
    pub session_id: String,
    pub entries: Vec<SessionEntry>,
    pub head: Option<SessionHead>,
    pub leaves: Vec<SessionEntry>,
    pub worker_leases: Vec<WorkerLease>,
    pub worker_runtimes: Vec<WorkerRuntimeRecord>,
    pub worker_queue: Vec<QueuedWorkerRequest>,
    pub usage: Vec<UsageLedgerRow>,
    pub reasons: Vec<BridgeEvent>,
    pub policy_limits: PolicyLimits,
    pub repository_divergence: RepositoryDivergence,
    pub completion: Option<crate::completion::CompletionSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryDivergence {
    pub status: String,
    pub selected_state: Option<serde_json::Value>,
    pub current_state: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize)]
    struct SnapshotNumbers {
        #[serde(serialize_with = "serialize_js_safe_i64")]
        signed: i64,
        #[serde(serialize_with = "serialize_optional_js_safe_i64")]
        optional: Option<i64>,
        #[serde(serialize_with = "serialize_js_safe_usize")]
        count: usize,
    }

    #[test]
    fn snapshot_serialization_rejects_javascript_unsafe_integers() {
        let safe = SnapshotNumbers {
            signed: bridge_protocol::MAX_SAFE_INTEGER,
            optional: Some(-bridge_protocol::MAX_SAFE_INTEGER),
            count: 3,
        };
        assert!(serde_json::to_value(safe).is_ok());

        for unsafe_number in [
            bridge_protocol::MAX_SAFE_INTEGER + 1,
            -(bridge_protocol::MAX_SAFE_INTEGER + 1),
        ] {
            let value = SnapshotNumbers {
                signed: unsafe_number,
                optional: None,
                count: 0,
            };
            let error = serde_json::to_value(value).unwrap_err();
            assert!(error.to_string().contains("safe integer"), "{error}");
        }

        let value = SnapshotNumbers {
            signed: 0,
            optional: Some(bridge_protocol::MAX_SAFE_INTEGER + 1),
            count: 0,
        };
        let error = serde_json::to_value(value).unwrap_err();
        assert!(error.to_string().contains("safe integer"), "{error}");

        if usize::BITS > 53 {
            let value = SnapshotNumbers {
                signed: 0,
                optional: None,
                count: bridge_protocol::MAX_SAFE_INTEGER as usize + 1,
            };
            let error = serde_json::to_value(value).unwrap_err();
            assert!(error.to_string().contains("safe integer"), "{error}");
        }
    }
}

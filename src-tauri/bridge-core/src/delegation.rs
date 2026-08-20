//! Typed delegation transport and worker-result protocol.
//!
//! Harness messages may still carry fenced `bridge-delegate` JSON, but parsing
//! immediately produces [`DelegationRequest`]. No free-form task/context object
//! crosses that boundary. Workers return a versioned [`WorkerResult`].

use crate::model;
pub use crate::model::CapabilityTier;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub const SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_MAX_DEPTH: i64 = 1;
pub const MAX_EVIDENCE_REFERENCES: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerRole {
    Research,
    Implementation,
    Verification,
    Planning,
    Documentation,
}

impl WorkerRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Research => "research",
            Self::Implementation => "implementation",
            Self::Verification => "verification",
            Self::Planning => "planning",
            Self::Documentation => "documentation",
        }
    }

    /// Fold what a model actually writes into one of the five roles.
    ///
    /// A model asked for a role writes `implementer`, `reviewer`, or
    /// `implementation-verifier`. Those are the same five jobs under different
    /// names, and rejecting the request over the spelling threw away real work.
    /// Anything genuinely unrecognized still returns `None` — this widens the
    /// vocabulary, it does not invent roles.
    pub fn parse(value: &str) -> Option<Self> {
        let normalized = value
            .trim()
            .to_ascii_lowercase()
            .replace(['-', ' ', '.'], "_");
        match normalized.as_str() {
            "research" | "researcher" | "investigation" | "investigator" | "analysis"
            | "analyst" | "explore" | "exploration" => Some(Self::Research),
            "implementation" | "implementer" | "implement" | "coder" | "code" | "coding"
            | "engineer" | "developer" | "dev" | "fix" | "builder" => Some(Self::Implementation),
            "verification" | "verifier" | "verify" | "implementation_verifier"
            | "implementation_verification" | "review" | "reviewer" | "test" | "tester"
            | "testing" | "qa" | "validation" | "validator" => Some(Self::Verification),
            "planning" | "planner" | "plan" | "design" | "designer" | "architect"
            | "architecture" | "orchestrator" | "coordinator" => Some(Self::Planning),
            "documentation" | "documenter" | "docs" | "doc" | "writer" | "technical_writer" => {
                Some(Self::Documentation)
            }
            _ => None,
        }
    }

    /// Whether a role's work is reading, not writing.
    ///
    /// This is authority, not a hint: the clamp in [`normalize_delegation`] uses
    /// it to hold a read-only role read-only no matter what write mode the model
    /// asked for.
    pub const fn is_read_only(self) -> bool {
        !matches!(self, Self::Implementation)
    }

    /// The write mode a role gets when the request does not name one.
    pub const fn default_write_mode(self) -> WriteMode {
        match self {
            Self::Implementation => WriteMode::Isolated,
            _ => WriteMode::ReadOnly,
        }
    }

    /// The result contract a role is held to. Derived host-side so the model
    /// never has to restate its own role in a second vocabulary.
    pub const fn output_contract(self) -> OutputContract {
        match self {
            Self::Research => OutputContract::ResearchResult,
            Self::Implementation => OutputContract::ImplementationResult,
            Self::Verification => OutputContract::VerificationResult,
            Self::Planning => OutputContract::DecisionResult,
            Self::Documentation => OutputContract::DocumentationResult,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WriteMode {
    ReadOnly,
    Shared,
    Isolated,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
}

impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutputContract {
    ImplementationResult,
    ResearchResult,
    VerificationResult,
    DecisionResult,
    DocumentationResult,
}

/// A delegation request as Bridge holds it.
///
/// Every transport field here is filled in by [`normalize_delegation`] before
/// deserialization, so a model only has to supply the semantic objective. The
/// struct stays strict — unknown fields are ignored rather than refused, because
/// an extra explanatory key from a model is not a reason to throw away the
/// request it was attached to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegationRequest {
    pub schema_version: u32,
    pub role: WorkerRole,
    pub objective: String,
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub known_facts: Vec<String>,
    #[serde(default)]
    pub decisions: Vec<String>,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
    #[serde(default)]
    pub relevant_files: Vec<String>,
    #[serde(default)]
    pub owned_paths: Vec<String>,
    pub write_mode: WriteMode,
    pub capability_tier: CapabilityTier,
    pub effort: Effort,
    /// A read-only worker is offline unless both this request and host policy opt in.
    #[serde(default)]
    pub network_access: bool,
    /// Logical artifact paths the worker may use under its assigned output directory.
    #[serde(default)]
    pub writable_output_paths: Vec<String>,
    #[serde(default)]
    pub verification: Vec<String>,
    pub output_contract: OutputContract,
    /// Temporary typed transport hint. Policy/capability discovery replaces it in #5/#6.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    /// Temporary runtime detail, never a durable routing semantic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl DelegationRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(format!(
                "unsupported delegation schema version {}",
                self.schema_version
            ));
        }
        require_non_empty("objective", &self.objective)?;
        require_non_empty_list("acceptanceCriteria", &self.acceptance_criteria)?;
        validate_non_empty_items("knownFacts", &self.known_facts)?;
        validate_non_empty_items("decisions", &self.decisions)?;
        validate_non_empty_items("evidenceIds", &self.evidence_ids)?;
        if self.evidence_ids.len() > MAX_EVIDENCE_REFERENCES {
            return Err(format!(
                "evidenceIds cannot contain more than {MAX_EVIDENCE_REFERENCES} entries"
            ));
        }
        let unique = self.evidence_ids.iter().collect::<HashSet<_>>();
        if unique.len() != self.evidence_ids.len() {
            return Err("evidenceIds cannot contain duplicates".into());
        }
        validate_non_empty_items("relevantFiles", &self.relevant_files)?;
        validate_non_empty_items("ownedPaths", &self.owned_paths)?;
        validate_non_empty_items("verification", &self.verification)?;
        validate_output_paths(&self.writable_output_paths)?;
        if let Some(harness) = &self.harness {
            if normalize_harness(harness).is_none() {
                return Err(format!("unsupported harness hint: {harness}"));
            }
        }
        Ok(())
    }

    pub fn runtime_harness(&self) -> String {
        self.harness
            .as_deref()
            .and_then(normalize_harness)
            .unwrap_or_else(|| "codex".into())
    }

    pub fn label(&self) -> String {
        format!(
            "{} · {}",
            role_label(self.role),
            self.capability_tier.as_str()
        )
    }
}

fn validate_output_paths(paths: &[String]) -> Result<(), String> {
    for path in paths {
        if path.is_empty()
            || path.starts_with('/')
            || path.split('/').any(|part| part == ".." || part.is_empty())
        {
            return Err(format!(
                "writableOutputPaths contains invalid relative path: {path}"
            ));
        }
    }
    Ok(())
}

fn role_label(role: WorkerRole) -> &'static str {
    match role {
        WorkerRole::Research => "Research",
        WorkerRole::Implementation => "Implementation",
        WorkerRole::Verification => "Verification",
        WorkerRole::Planning => "Planning",
        WorkerRole::Documentation => "Documentation",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerResultStatus {
    Completed,
    Failed,
    Cancelled,
    Blocked,
    NeedsDelegation,
    /// The worker's final message could not be read as a result, even after
    /// normalization.
    ///
    /// Separate from `Failed` on purpose. "You formatted the envelope wrong" and
    /// "the task did not work" are different facts, and conflating them was what
    /// turned a bad fence into a failed task, a failed task into a retry, and a
    /// retry into another paid turn for a cause that had not changed.
    ProtocolInvalid,
}

impl WorkerResultStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Blocked => "blocked",
            Self::NeedsDelegation => "needs_delegation",
            Self::ProtocolInvalid => "protocol_invalid",
        }
    }

    /// Fold what a model writes into a status.
    ///
    /// `escalate` means `needs_delegation`; `success` means `completed`. A worker
    /// that finished its job and said so in its own words has still finished it.
    pub fn parse(value: &str) -> Option<Self> {
        let normalized = value
            .trim()
            .to_ascii_lowercase()
            .replace(['-', ' ', '.'], "_");
        match normalized.as_str() {
            "completed" | "complete" | "success" | "succeeded" | "done" | "ok" | "finished" => {
                Some(Self::Completed)
            }
            "failed" | "failure" | "fail" | "error" | "errored" => Some(Self::Failed),
            "cancelled" | "canceled" | "aborted" | "abandoned" => Some(Self::Cancelled),
            "blocked" | "block" | "stuck" | "waiting" | "needs_input" | "needs_approval" => {
                Some(Self::Blocked)
            }
            "needs_delegation" | "needsdelegation" | "escalate" | "escalation" | "delegate"
            | "needs_specialist" | "handoff" => Some(Self::NeedsDelegation),
            "protocol_invalid" => Some(Self::ProtocolInvalid),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TestStatus {
    Passed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerTestResult {
    pub command: String,
    pub status: TestStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestedNextAction {
    Finish,
    Retry,
    FollowUp,
    RequestApproval,
}

impl SuggestedNextAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Finish => "finish",
            Self::Retry => "retry",
            Self::FollowUp => "follow_up",
            Self::RequestApproval => "request_approval",
        }
    }

    /// Fold a model's suggestion into the four Bridge acts on. Advisory only —
    /// Rust decides what actually happens next, so a generous reading here
    /// cannot buy the model an action it is not entitled to.
    pub fn parse(value: &str) -> Option<Self> {
        let normalized = value
            .trim()
            .to_ascii_lowercase()
            .replace(['-', ' ', '.'], "_");
        match normalized.as_str() {
            "finish" | "finished" | "done" | "complete" | "completed" | "none" | "stop" => {
                Some(Self::Finish)
            }
            "retry" | "retry_once" | "try_again" => Some(Self::Retry),
            "follow_up" | "followup" | "continue" | "next" | "escalate"
            | "delegate_implementation" | "delegate" | "delegation" | "handoff" => {
                Some(Self::FollowUp)
            }
            "request_approval" | "approval" | "ask" | "ask_user" | "needs_approval"
            | "request_permission" => Some(Self::RequestApproval),
            _ => None,
        }
    }

    /// What Bridge assumes when a result does not say. Derived from the status,
    /// which the worker did report, rather than demanded a second time.
    pub const fn for_status(status: WorkerResultStatus) -> Self {
        match status {
            WorkerResultStatus::Completed | WorkerResultStatus::Cancelled => Self::Finish,
            WorkerResultStatus::Failed | WorkerResultStatus::ProtocolInvalid => Self::FollowUp,
            WorkerResultStatus::Blocked => Self::RequestApproval,
            WorkerResultStatus::NeedsDelegation => Self::FollowUp,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerResult {
    pub schema_version: u32,
    pub status: WorkerResultStatus,
    pub summary: String,
    #[serde(default)]
    pub files_changed: Vec<String>,
    #[serde(default)]
    pub tests: Vec<WorkerTestResult>,
    #[serde(default)]
    pub decisions: Vec<String>,
    #[serde(default)]
    pub risks: Vec<String>,
    #[serde(default)]
    pub remaining_work: Vec<String>,
    pub suggested_next_action: SuggestedNextAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_role: Option<WorkerRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_task: Option<String>,
}

impl WorkerResult {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(format!(
                "unsupported worker-result schema version {}",
                self.schema_version
            ));
        }
        require_non_empty("summary", &self.summary)?;
        validate_non_empty_items("filesChanged", &self.files_changed)?;
        validate_non_empty_items("decisions", &self.decisions)?;
        validate_non_empty_items("risks", &self.risks)?;
        validate_non_empty_items("remainingWork", &self.remaining_work)?;
        for test in &self.tests {
            require_non_empty("tests.command", &test.command)?;
        }
        if self.status == WorkerResultStatus::NeedsDelegation {
            // `suggestedRole` is derived host-side when absent. A worker must not
            // have to guess a closed vocabulary word to avoid losing the work it
            // already did — naming what it needs is enough.
            require_non_empty(
                "suggestedTask",
                self.suggested_task.as_deref().unwrap_or_default(),
            )?;
        }
        Ok(())
    }

    /// A formatting mistake is never a retryable task failure: nothing about the
    /// task changed, so another turn would produce the same thing at the same
    /// price. See [`WorkerResultStatus::ProtocolInvalid`].
    pub fn is_retryable(&self) -> bool {
        self.status == WorkerResultStatus::Failed
    }

    /// Whether this result describes a transport problem rather than the work.
    pub fn is_protocol_invalid(&self) -> bool {
        self.status == WorkerResultStatus::ProtocolInvalid
    }

    pub fn is_terminal_cancellation(&self) -> bool {
        self.status == WorkerResultStatus::Cancelled
    }
}

fn require_non_empty(field: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{field} must not be empty"))
    } else {
        Ok(())
    }
}

fn require_non_empty_list(field: &str, values: &[String]) -> Result<(), String> {
    if values.is_empty() {
        return Err(format!("{field} must contain at least one item"));
    }
    validate_non_empty_items(field, values)
}

fn validate_non_empty_items(field: &str, values: &[String]) -> Result<(), String> {
    if values.iter().any(|value| value.trim().is_empty()) {
        Err(format!("{field} must not contain empty items"))
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseOutcome<T> {
    Absent,
    Parsed(T),
    Invalid { raw: String, reason: String },
}

#[derive(Debug, Deserialize)]
struct LegacyDirective {
    harness: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    effort: Option<String>,
    task: String,
    #[serde(default)]
    context: Option<String>,
}

impl LegacyDirective {
    fn into_typed(self) -> Result<DelegationRequest, String> {
        let Self {
            harness,
            model,
            effort,
            task,
            context,
        } = self;
        let harness =
            normalize_harness(&harness).ok_or_else(|| format!("unsupported harness: {harness}"))?;
        let objective = task.trim().to_owned();
        require_non_empty("task", &objective)?;
        let known_facts = context
            .map(|context| context.trim().to_owned())
            .filter(|context| !context.is_empty())
            .into_iter()
            .collect();
        let request = DelegationRequest {
            schema_version: SCHEMA_VERSION,
            role: WorkerRole::Implementation,
            objective,
            acceptance_criteria: vec![
                "Complete the objective and report concrete verification evidence".into(),
            ],
            known_facts,
            decisions: Vec::new(),
            evidence_ids: Vec::new(),
            relevant_files: Vec::new(),
            owned_paths: Vec::new(),
            write_mode: WriteMode::Shared,
            capability_tier: CapabilityTier::Standard,
            effort: parse_effort(effort.as_deref().unwrap_or("medium")),
            network_access: false,
            writable_output_paths: Vec::new(),
            verification: Vec::new(),
            output_contract: OutputContract::ImplementationResult,
            model: model.map(|model| model.trim().to_ascii_lowercase()),
            harness: Some(harness),
        };
        request.validate()?;
        Ok(request)
    }
}

struct FencedBlock {
    body: String,
}

fn fenced_blocks(text: &str, matches_tag: impl Fn(&str) -> bool) -> Vec<FencedBlock> {
    let mut blocks = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("```") {
            continue;
        }
        let tag = trimmed.trim_start_matches('`').trim().to_ascii_lowercase();
        if !matches_tag(&tag) {
            continue;
        }
        let mut body = String::new();
        for inner in lines.by_ref() {
            if inner.trim_start().starts_with("```") {
                break;
            }
            body.push_str(inner);
            body.push('\n');
        }
        blocks.push(FencedBlock {
            body: body.trim().to_owned(),
        });
    }
    blocks
}

fn is_delegation_tag(tag: &str) -> bool {
    tag.contains("bridge") && tag.contains("delegate")
}

fn is_worker_result_tag(tag: &str) -> bool {
    tag.contains("bridge") && tag.contains("worker") && tag.contains("result")
}

/// What the host filled in, corrected, or ignored on the model's behalf.
///
/// Recorded rather than applied silently: a normalized envelope has to be
/// auditable, and "Bridge chose isolated because the role is implementation" is
/// a fact someone reading a session later needs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Normalizations(Vec<String>);

impl Normalizations {
    fn record(&mut self, note: impl Into<String>) {
        self.0.push(note.into());
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn notes(&self) -> &[String] {
        &self.0
    }

    pub fn summary(&self) -> String {
        self.0.join("; ")
    }

    fn absorb(&mut self, other: Normalizations) {
        self.0.extend(other.0);
    }
}

/// The fields a delegation request owns semantically. Everything else in the
/// envelope is transport, and transport is Bridge's job.
const DELEGATION_SEMANTIC_FIELDS: &[&str] = &[
    "role",
    "objective",
    "acceptanceCriteria",
    "knownFacts",
    "decisions",
    "evidenceIds",
    "relevantFiles",
    "ownedPaths",
    "verification",
    "harness",
    "model",
];

/// Fill in a delegation envelope's transport fields and fold its vocabulary.
///
/// This is the authority boundary made concrete. The model supplies an objective
/// and what it knows; Bridge supplies the schema version, the defaults, the
/// derived output contract — and, crucially, **clamps** the write mode by role,
/// so a generous reading of the wire format never buys a read-only role a
/// writable worker. Opening the format does not open the permissions.
pub fn normalize_delegation(value: &mut Value) -> Normalizations {
    let mut notes = Normalizations::default();
    let Some(object) = value.as_object_mut() else {
        return notes;
    };

    match object.get("schemaVersion").and_then(Value::as_u64) {
        Some(version) if version == SCHEMA_VERSION as u64 => {}
        Some(version) => {
            notes.record(format!("schemaVersion {version} rewritten to {SCHEMA_VERSION}"));
            object.insert("schemaVersion".into(), Value::from(SCHEMA_VERSION));
        }
        None => {
            object.insert("schemaVersion".into(), Value::from(SCHEMA_VERSION));
        }
    }

    // Role first: every other default is derived from it.
    let role = object
        .get("role")
        .and_then(Value::as_str)
        .and_then(|raw| {
            let role = WorkerRole::parse(raw);
            if role.is_some_and(|role| role.as_str() != raw) {
                notes.record(format!("role {raw:?} read as {}", role.unwrap().as_str()));
            }
            role
        })
        .unwrap_or_else(|| {
            notes.record("role missing or unreadable; defaulted to implementation");
            WorkerRole::Implementation
        });
    object.insert("role".into(), Value::from(role.as_str()));

    if object
        .get("objective")
        .and_then(Value::as_str)
        .is_none_or(|objective| objective.trim().is_empty())
    {
        // The one thing Bridge cannot invent. Left absent so validation refuses
        // it with a reason instead of dispatching a worker at nothing.
        notes.record("objective missing");
    }

    let criteria_present = object
        .get("acceptanceCriteria")
        .and_then(Value::as_array)
        .is_some_and(|criteria| !criteria.is_empty());
    if !criteria_present {
        object.insert(
            "acceptanceCriteria".into(),
            Value::from(vec![Value::from(
                "Complete the objective and report concrete verification evidence",
            )]),
        );
        notes.record("acceptanceCriteria defaulted");
    }

    // Write mode: normalized, then clamped by role. The clamp is the authority.
    let requested_mode = object.get("writeMode").and_then(Value::as_str).map(str::to_owned);
    let mode = match requested_mode.as_deref().and_then(parse_write_mode) {
        Some(mode) => mode,
        None => {
            if let Some(raw) = &requested_mode {
                notes.record(format!("writeMode {raw:?} unreadable; defaulted by role"));
            }
            role.default_write_mode()
        }
    };
    let clamped = if role.is_read_only() && mode != WriteMode::ReadOnly {
        notes.record(format!(
            "writeMode {} clamped to readOnly for a {} worker",
            write_mode_wire_name(mode),
            role.as_str()
        ));
        WriteMode::ReadOnly
    } else {
        mode
    };
    object.insert("writeMode".into(), Value::from(write_mode_wire_name(clamped)));

    let tier = match object.get("capabilityTier").and_then(Value::as_str) {
        Some(raw) => parse_capability_tier(raw).unwrap_or_else(|| {
            notes.record(format!("capabilityTier {raw:?} unreadable; defaulted to standard"));
            CapabilityTier::Standard
        }),
        None => CapabilityTier::Standard,
    };
    object.insert("capabilityTier".into(), Value::from(tier.as_str()));

    let effort = match object.get("effort").and_then(Value::as_str) {
        Some(raw) => parse_effort(raw),
        None => Effort::Medium,
    };
    object.insert("effort".into(), Value::from(effort.as_str()));

    // Derived, never demanded: the contract is a function of the role, and
    // making the model restate it in a second vocabulary only created a way to
    // disagree with itself.
    let contract = role.output_contract();
    if object
        .get("outputContract")
        .and_then(Value::as_str)
        .is_some_and(|raw| raw != output_contract_wire_name(contract))
    {
        notes.record(format!(
            "outputContract derived from role as {}",
            output_contract_wire_name(contract)
        ));
    }
    object.insert(
        "outputContract".into(),
        Value::from(output_contract_wire_name(contract)),
    );

    let ignored = object
        .keys()
        .filter(|key| {
            !DELEGATION_SEMANTIC_FIELDS.contains(&key.as_str())
                && !matches!(
                    key.as_str(),
                    "schemaVersion"
                        | "writeMode"
                        | "capabilityTier"
                        | "effort"
                        | "outputContract"
                        | "networkAccess"
                        | "writableOutputPaths"
                )
        })
        .cloned()
        .collect::<Vec<_>>();
    if !ignored.is_empty() {
        // Named, not fatal. An extra explanatory key is the model being helpful;
        // discarding the whole request over it was the bug.
        notes.record(format!("ignored extra field(s): {}", ignored.join(", ")));
        for key in ignored {
            object.remove(&key);
        }
    }
    notes
}

/// Fill in a worker result's transport fields and fold its vocabulary.
///
/// Same boundary from the other direction: the worker reports what happened, and
/// Bridge supplies the schema version, the next action, and the escalation role
/// it would otherwise have had to guess.
pub fn normalize_worker_result(value: &mut Value) -> Normalizations {
    let mut notes = Normalizations::default();
    let Some(object) = value.as_object_mut() else {
        return notes;
    };

    match object.get("schemaVersion").and_then(Value::as_u64) {
        Some(version) if version == SCHEMA_VERSION as u64 => {}
        Some(version) => {
            notes.record(format!("schemaVersion {version} rewritten to {SCHEMA_VERSION}"));
            object.insert("schemaVersion".into(), Value::from(SCHEMA_VERSION));
        }
        None => {
            object.insert("schemaVersion".into(), Value::from(SCHEMA_VERSION));
        }
    }

    let status = object.get("status").and_then(Value::as_str).and_then(|raw| {
        let status = WorkerResultStatus::parse(raw);
        if status.is_some_and(|status| status.as_str() != raw) {
            notes.record(format!(
                "status {raw:?} read as {}",
                status.unwrap().as_str()
            ));
        }
        status
    });
    if let Some(status) = status {
        object.insert("status".into(), Value::from(status.as_str()));
    }

    // A summary under another name is still a summary.
    if object
        .get("summary")
        .and_then(Value::as_str)
        .is_none_or(|summary| summary.trim().is_empty())
    {
        let borrowed = ["result", "message", "details", "detail", "output", "text"]
            .into_iter()
            .find_map(|key| {
                object
                    .get(key)
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .map(str::to_owned)
            });
        if let Some(borrowed) = borrowed {
            notes.record("summary taken from an equivalent field");
            object.insert("summary".into(), Value::from(borrowed));
        }
    }

    let action = match object.get("suggestedNextAction").and_then(Value::as_str) {
        Some(raw) => SuggestedNextAction::parse(raw).unwrap_or_else(|| {
            let derived = SuggestedNextAction::for_status(status.unwrap_or(WorkerResultStatus::Completed));
            notes.record(format!(
                "suggestedNextAction {raw:?} read as {}",
                derived.as_str()
            ));
            derived
        }),
        None => SuggestedNextAction::for_status(status.unwrap_or(WorkerResultStatus::Completed)),
    };
    object.insert("suggestedNextAction".into(), Value::from(action.as_str()));

    if status == Some(WorkerResultStatus::NeedsDelegation) {
        let derived = object
            .get("suggestedRole")
            .and_then(Value::as_str)
            .and_then(WorkerRole::parse)
            .unwrap_or_else(|| {
                // Derived from what the worker asked for, not demanded from it.
                // Having to invent a closed vocabulary word to avoid losing
                // completed work is exactly the trap this removes.
                let task = object
                    .get("suggestedTask")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let role = WorkerRole::parse(task).unwrap_or_else(|| infer_role_from_task(task));
                notes.record(format!("suggestedRole derived as {}", role.as_str()));
                role
            });
        object.insert("suggestedRole".into(), Value::from(derived.as_str()));
    } else if let Some(raw) = object.get("suggestedRole").and_then(Value::as_str) {
        match WorkerRole::parse(raw) {
            Some(role) => {
                object.insert("suggestedRole".into(), Value::from(role.as_str()));
            }
            None => {
                // Advisory on a result that is not escalating: drop it rather
                // than fail an otherwise usable result over it.
                notes.record(format!("ignored unreadable suggestedRole {raw:?}"));
                object.remove("suggestedRole");
            }
        }
    }

    if let Some(tests) = object.get_mut("tests").and_then(Value::as_array_mut) {
        for test in tests.iter_mut() {
            let Some(test) = test.as_object_mut() else {
                continue;
            };
            let status = match test.get("status").and_then(Value::as_str) {
                Some(raw) => parse_test_status(raw).unwrap_or(TestStatus::Skipped),
                None => TestStatus::Skipped,
            };
            test.insert("status".into(), Value::from(test_status_wire_name(status)));
        }
    }
    notes
}

fn infer_role_from_task(task: &str) -> WorkerRole {
    let task = task.to_ascii_lowercase();
    for (needles, role) in [
        (["verify", "test", "review"], WorkerRole::Verification),
        (["research", "investigate", "find out"], WorkerRole::Research),
        (["document", "docs", "changelog"], WorkerRole::Documentation),
        (["plan", "design", "decide"], WorkerRole::Planning),
    ] {
        if needles.iter().any(|needle| task.contains(needle)) {
            return role;
        }
    }
    WorkerRole::Implementation
}

fn parse_write_mode(value: &str) -> Option<WriteMode> {
    let normalized = value
        .trim()
        .to_ascii_lowercase()
        .replace(['-', ' ', '.'], "_");
    match normalized.as_str() {
        "readonly" | "read_only" | "read" | "none" | "no_write" => Some(WriteMode::ReadOnly),
        "shared" | "workspace" | "write" => Some(WriteMode::Shared),
        "isolated" | "worktree" | "branch" => Some(WriteMode::Isolated),
        "full" | "danger_full_access" | "unrestricted" => Some(WriteMode::Full),
        _ => None,
    }
}

pub const fn write_mode_wire_name(mode: WriteMode) -> &'static str {
    match mode {
        WriteMode::ReadOnly => "readOnly",
        WriteMode::Shared => "shared",
        WriteMode::Isolated => "isolated",
        WriteMode::Full => "full",
    }
}

const fn output_contract_wire_name(contract: OutputContract) -> &'static str {
    match contract {
        OutputContract::ImplementationResult => "implementation-result",
        OutputContract::ResearchResult => "research-result",
        OutputContract::VerificationResult => "verification-result",
        OutputContract::DecisionResult => "decision-result",
        OutputContract::DocumentationResult => "documentation-result",
    }
}

fn parse_capability_tier(value: &str) -> Option<CapabilityTier> {
    match value.trim().to_ascii_lowercase().as_str() {
        "fast" | "low" | "cheap" | "quick" => Some(CapabilityTier::Fast),
        "standard" | "medium" | "balanced" | "default" => Some(CapabilityTier::Standard),
        "strong" | "high" | "max" | "best" => Some(CapabilityTier::Strong),
        _ => None,
    }
}

fn parse_test_status(value: &str) -> Option<TestStatus> {
    match value.trim().to_ascii_lowercase().as_str() {
        "passed" | "pass" | "passing" | "green" | "ok" | "success" => Some(TestStatus::Passed),
        "failed" | "fail" | "failing" | "red" | "error" => Some(TestStatus::Failed),
        "skipped" | "skip" | "not_run" | "notrun" | "n/a" => Some(TestStatus::Skipped),
        _ => None,
    }
}

const fn test_status_wire_name(status: TestStatus) -> &'static str {
    match status {
        TestStatus::Passed => "passed",
        TestStatus::Failed => "failed",
        TestStatus::Skipped => "skipped",
    }
}

/// Parsed requests plus what Bridge filled in or corrected to get them.
///
/// The notes travel with the requests rather than being applied silently: a
/// normalized envelope has to be auditable, and "Bridge clamped writeMode to
/// readOnly because the role is research" is a fact a session's reader needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedRequests {
    pub requests: Vec<DelegationRequest>,
    pub notes: Normalizations,
}

pub fn parse_delegation_requests(text: &str) -> ParseOutcome<NormalizedRequests> {
    let blocks = fenced_blocks(text, is_delegation_tag);
    if blocks.is_empty() {
        return ParseOutcome::Absent;
    }
    let raw = blocks
        .iter()
        .map(|block| block.body.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let mut requests = Vec::new();
    let mut notes = Normalizations::default();
    for block in blocks {
        let value = match serde_json::from_str::<Value>(&block.body) {
            Ok(value) => value,
            Err(error) => {
                return ParseOutcome::Invalid {
                    raw,
                    reason: format!("invalid delegation JSON: {error}"),
                }
            }
        };
        let values = match value {
            Value::Array(values) => values,
            value => vec![value],
        };
        for value in values {
            match request_from_value(value) {
                Ok((request, request_notes)) => {
                    requests.push(request);
                    notes.absorb(request_notes);
                }
                Err(reason) => return ParseOutcome::Invalid { raw, reason },
            }
        }
    }
    if requests.is_empty() {
        ParseOutcome::Invalid {
            raw,
            reason: "delegation block contained no requests".into(),
        }
    } else {
        ParseOutcome::Parsed(NormalizedRequests { requests, notes })
    }
}

fn request_from_value(mut value: Value) -> Result<(DelegationRequest, Normalizations), String> {
    if value.get("objective").is_some() {
        // Normalize before deserializing: the model owns the objective, Bridge
        // owns the envelope.
        let notes = normalize_delegation(&mut value);
        let request: DelegationRequest =
            serde_json::from_value(value).map_err(|error| error.to_string())?;
        request.validate()?;
        Ok((request, notes))
    } else {
        let request = serde_json::from_value::<LegacyDirective>(value)
            .map_err(|error| error.to_string())?
            .into_typed()?;
        Ok((request, Normalizations::default()))
    }
}

pub fn parse_worker_result(text: &str) -> ParseOutcome<WorkerResult> {
    let blocks = fenced_blocks(text, is_worker_result_tag);
    if blocks.is_empty() {
        return ParseOutcome::Absent;
    }
    if blocks.len() != 1 {
        return ParseOutcome::Invalid {
            raw: blocks
                .iter()
                .map(|block| block.body.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            reason: "expected exactly one bridge-worker-result block".into(),
        };
    }
    let raw = blocks[0].body.clone();
    let mut value = match serde_json::from_str::<Value>(&raw) {
        Ok(value) => value,
        Err(error) => {
            return ParseOutcome::Invalid {
                raw,
                reason: format!("invalid worker-result JSON: {error}"),
            }
        }
    };
    // Same boundary as delegation: fold the vocabulary and fill the transport
    // fields before deserializing, so a spelling difference is not a lost result.
    normalize_worker_result(&mut value);
    let result = match serde_json::from_value::<WorkerResult>(value) {
        Ok(result) => result,
        Err(error) => {
            return ParseOutcome::Invalid {
                raw,
                reason: format!("invalid worker-result JSON: {error}"),
            }
        }
    };
    match result.validate() {
        Ok(()) => ParseOutcome::Parsed(result),
        Err(reason) => ParseOutcome::Invalid { raw, reason },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerOutputDisposition {
    Structured(WorkerResult),
    RequestRepair { prompt: String, reason: String },
    Unstructured { raw: String, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerOutputAction {
    AwaitingRepair { reason: String },
    Structured(WorkerResult),
    Unstructured { raw: String, reason: String },
}

#[derive(Debug, Default)]
pub struct ResultRepairTracker {
    first_invalid_output: HashMap<String, String>,
}

impl ResultRepairTracker {
    pub fn evaluate(&mut self, session_id: &str, text: &str) -> WorkerOutputDisposition {
        match parse_worker_result(text) {
            ParseOutcome::Parsed(result) => {
                self.first_invalid_output.remove(session_id);
                WorkerOutputDisposition::Structured(result)
            }
            ParseOutcome::Absent => self.invalid(
                session_id,
                text,
                "missing bridge-worker-result block".into(),
            ),
            ParseOutcome::Invalid { reason, .. } => self.invalid(session_id, text, reason),
        }
    }

    pub fn process(
        &mut self,
        session_id: &str,
        text: &str,
        send_same_session_repair: impl FnOnce(&str) -> bool,
    ) -> WorkerOutputAction {
        match self.evaluate(session_id, text) {
            WorkerOutputDisposition::Structured(result) => WorkerOutputAction::Structured(result),
            WorkerOutputDisposition::Unstructured { raw, reason } => {
                WorkerOutputAction::Unstructured { raw, reason }
            }
            WorkerOutputDisposition::RequestRepair { prompt, reason } => {
                if send_same_session_repair(&prompt) {
                    WorkerOutputAction::AwaitingRepair { reason }
                } else {
                    self.first_invalid_output.remove(session_id);
                    WorkerOutputAction::Unstructured {
                        raw: text.to_owned(),
                        reason: format!("{reason}; same-session repair could not be delivered"),
                    }
                }
            }
        }
    }

    fn invalid(&mut self, session_id: &str, raw: &str, reason: String) -> WorkerOutputDisposition {
        if let Some(first) = self.first_invalid_output.remove(session_id) {
            return WorkerOutputDisposition::Unstructured {
                raw: format!("Initial invalid output:\n{first}\n\nInvalid repair output:\n{raw}"),
                reason,
            };
        }
        self.first_invalid_output
            .insert(session_id.to_owned(), raw.to_owned());
        WorkerOutputDisposition::RequestRepair {
            prompt: worker_result_repair_prompt(&reason),
            reason,
        }
    }
}

/// How much of an unreadable worker message is carried forward as evidence.
/// Enough to be worth reading; never an unbounded transcript pasted into a
/// parent's context.
pub const MAX_PRESERVED_PROSE_BYTES: usize = 1_200;

/// A result for output Bridge could not read, with the worker's own words kept.
///
/// The old behaviour reported `failed` with "the raw worker response was
/// excluded from parent context" — which threw away possibly-good work and told
/// the orchestrator the task had failed, so it retried a task that may have
/// succeeded. This says what actually happened, and keeps the prose.
pub fn protocol_invalid_result(raw: &str, reason: &str) -> WorkerResult {
    let prose = strip_worker_result(raw);
    let prose = prose.trim();
    let excerpt = if prose.is_empty() {
        None
    } else {
        Some(truncate_on_char_boundary(prose, MAX_PRESERVED_PROSE_BYTES))
    };
    WorkerResult {
        schema_version: SCHEMA_VERSION,
        status: WorkerResultStatus::ProtocolInvalid,
        summary: match &excerpt {
            Some(excerpt) => format!(
                "The worker's result could not be read ({reason}). Its own words, unverified:\n\n{excerpt}"
            ),
            None => format!("The worker's result could not be read ({reason}), and it left no prose."),
        },
        files_changed: Vec::new(),
        tests: Vec::new(),
        decisions: Vec::new(),
        risks: vec![
            "This is a transport failure, not a task outcome: nothing here has been verified"
                .into(),
        ],
        remaining_work: vec![
            "Confirm what the worker actually did before treating this objective as done".into(),
        ],
        suggested_next_action: SuggestedNextAction::FollowUp,
        suggested_role: None,
        suggested_task: None,
    }
}

fn truncate_on_char_boundary(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut cut = max_bytes;
    while !value.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…", &value[..cut])
}

pub fn worker_result_repair_prompt(reason: &str) -> String {
    format!(
        r#"Your previous final output could not be parsed ({reason}). This is your one repair turn. Do not perform more work. Return exactly one fenced `bridge-worker-result` JSON object matching schemaVersion 1 with: status, summary, filesChanged, tests, decisions, risks, remainingWork, suggestedNextAction, and optional suggestedRole/suggestedTask. Do not add prose outside the fence."#
    )
}

/// Feedback injected back into the orchestrator when a `bridge-delegate`
/// request is rejected before any worker starts. Without this the request is
/// silently dropped and the orchestrator goes idle, which reads to the user as
/// "the subagent returned no results". The message names the failing reason and
/// the exact accepted vocabulary so the orchestrator can re-emit a valid request.
pub fn invalid_request_feedback(reason: &str) -> String {
    // Deliberately shorter than it was. Bridge now fills in schemaVersion,
    // writeMode, capabilityTier, effort, and outputContract, and folds role
    // spellings — so listing that whole vocabulary back at the model was
    // teaching it to author fields it does not own. What is left is what only
    // the orchestrator can supply.
    format!(
        r#"Your last `bridge-delegate` request could not be used, so no worker started and no result is coming: {reason}.

Re-emit one corrected `bridge-delegate` JSON object. You only need to supply the meaning of the work:
- objective: what the worker must accomplish (required, non-empty)
- role: research | implementation | verification | planning | documentation (common synonyms are understood)
- acceptanceCriteria, knownFacts, decisions, relevantFiles, ownedPaths, verification: optional context

Bridge supplies the schema version, write mode, capability tier, effort, and output contract, and enforces path scope and permissions regardless of what a request asks for. Do not restate this guidance to the user."#
    )
}

pub fn strip_directives(text: &str) -> String {
    strip_machine_blocks(text, is_delegation_tag)
}

pub fn strip_worker_result(text: &str) -> String {
    strip_machine_blocks(text, is_worker_result_tag)
}

fn strip_machine_blocks(text: &str, matches_tag: impl Fn(&str) -> bool) -> String {
    let lines = text.lines().collect::<Vec<_>>();
    let mut kept = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            let tag = trimmed.trim_start_matches('`').trim().to_ascii_lowercase();
            if matches_tag(&tag) {
                let closing = ((index + 1)..lines.len())
                    .find(|candidate| lines[*candidate].trim_start().starts_with("```"));
                if let Some(closing) = closing {
                    index = closing + 1;
                    continue;
                }
                // An unclosed block is malformed, so preserve it verbatim
                // rather than deleting all following user-visible prose.
                kept.extend_from_slice(&lines[index..]);
                break;
            }
        }
        kept.push(line);
        index += 1;
    }
    kept.join("\n").trim().to_owned()
}

pub fn protocol(depth: i64) -> String {
    if depth >= DEFAULT_MAX_DEPTH {
        // A bounded request, not a prohibition. The old wording told the worker
        // it could not delegate and then required an exact closed-vocabulary
        // value to say it needed help — so a worker that had done real work
        // could lose all of it over one word. Bridge derives the role now; the
        // worker only has to describe what it needs.
        return r#"## Bridge worker topology

You are a depth-one worker: you do not spawn workers yourself. You may ask for **one** focused specialist through your result — return `status: "needs_delegation"` with a `suggestedTask` describing what is needed. `suggestedRole` is optional; Bridge derives it. The parent and the Rust policy gate decide whether the specialist runs, under the same depth, path-scope, and budget limits that apply to you.

Always report the work you already completed in the same result. Asking for help never means discarding your own findings."#
            .into();
    }
    r#"## Delegating work (Bridge typed protocol v1)

Delegate only focused, non-trivial work. Emit one fenced `bridge-delegate` JSON object using this schema:

```bridge-delegate
{"schemaVersion":1,"role":"implementation","objective":"Add refresh-token rotation","acceptanceCriteria":["Old refresh tokens become invalid","Existing auth tests remain green"],"knownFacts":[],"decisions":["Use the existing SQLite token store"],"evidenceIds":[],"relevantFiles":["src/auth/store.rs"],"ownedPaths":["src/auth/**"],"writeMode":"isolated","capabilityTier":"standard","effort":"medium","verification":["cargo test auth"],"outputContract":"implementation-result","harness":"codex"}
```

After emitting a request, stop and wait. Default topology is flat: the worker cannot directly spawn another worker. Do trivial work in the parent."#
        .into()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerEvidence {
    pub evidence_id: String,
    pub child_session_id: String,
    pub result: WorkerResult,
}

pub fn worker_contract(role: WorkerRole, depth: i64) -> String {
    format!(
        r#"You are a Bridge {role:?} worker assigned one focused objective.

Complete only the supplied objective. You do not spawn workers yourself; if you need one focused specialist, say so in your result with `status: "needs_delegation"` and a `suggestedTask`.

End with exactly one fenced `bridge-worker-result` JSON object matching schemaVersion 1:

```bridge-worker-result
{{"schemaVersion":1,"status":"completed","summary":"What changed or was found","filesChanged":[],"tests":[{{"command":"command run","status":"passed"}}],"decisions":[],"risks":[],"remainingWork":[],"suggestedNextAction":"finish"}}
```

{}"#,
        protocol(depth)
    )
}

pub fn worker_task_context(
    request: &DelegationRequest,
    branch: &str,
    evidence: &[WorkerEvidence],
) -> String {
    let criteria = bullet_list(&request.acceptance_criteria);
    let facts = bullet_list_or_none(&request.known_facts);
    let decisions = bullet_list_or_none(&request.decisions);
    let files = bullet_list_or_none(&request.relevant_files);
    let owned = bullet_list_or_none(&request.owned_paths);
    let verification = bullet_list_or_none(&request.verification);
    let evidence = if evidence.is_empty() {
        "- None available".into()
    } else {
        evidence
            .iter()
            .map(|item| {
                format!(
                    "- Evidence ID `{}` from worker `{}`:\n```json\n{}\n```",
                    item.evidence_id,
                    item.child_session_id,
                    serde_json::to_string(&item.result)
                        .expect("validated worker evidence always serializes")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        r#"Branch: `{branch}`.

## Objective
{objective}

## Acceptance criteria
{criteria}

## Known facts
{facts}

## Locked decisions
{decisions}

## Prior worker evidence (canonical SQLite records)
{evidence}

## Relevant files
{files}

## Owned paths
{owned}

Write mode: {write_mode:?}. Capability tier: {tier:?}. Effort: {effort}.

## Verification
{verification}"#,
        objective = request.objective,
        write_mode = request.write_mode,
        tier = request.capability_tier,
        effort = request.effort.as_str(),
    )
}

pub fn worker_briefing(
    request: &DelegationRequest,
    depth: i64,
    branch: &str,
    evidence: &[WorkerEvidence],
) -> String {
    format!(
        "{}\n\n{}\n\n{}",
        worker_contract(request.role, depth),
        worker_task_context(request, branch, evidence),
        crate::prompts::RENDERING_NOTE,
    )
}

fn bullet_list(items: &[String]) -> String {
    items
        .iter()
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn bullet_list_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "- None provided".into()
    } else {
        bullet_list(items)
    }
}

/// Fold a harness hint written by a model into a canonical harness id.
///
/// The alias arms exist because a model writes "Claude Code" or "anthropic"
/// when it means `claude`. Any other well-formed agent id passes through
/// unchanged — an installed agent is named by its id and has no aliases to
/// fold.
///
/// `shell` is refused despite being a valid harness id. Admitting it would let
/// a delegation directive spawn a shell worker where it previously could not,
/// and widening what a model may delegate to is a product decision, not a side
/// effect of opening an identifier.
pub fn normalize_harness(value: &str) -> Option<String> {
    let lowercased = value.trim().to_ascii_lowercase();
    match lowercased.as_str() {
        "claude" | "claude-code" | "claudecode" | "anthropic" => Some("claude".into()),
        "codex" | "gpt" | "openai" => Some("codex".into()),
        "opencode" | "open-code" => Some("opencode".into()),
        "shell" => None,
        candidate => model::Harness::parse(candidate)
            .ok()
            .map(|harness| harness.id().into_owned()),
    }
}

pub fn model_display(model: &str) -> String {
    match model {
        "sonnet" => "Sonnet",
        "opus" => "Opus",
        "haiku" => "Haiku",
        "fable" => "Fable",
        "gpt-5.6-luna" => "GPT Luna",
        "gpt-5.6-terra" => "GPT Terra",
        "gpt-5.6-sol" => "GPT Sol",
        "gpt-5.3-codex" => "GPT-5.3 Codex",
        other => other,
    }
    .to_owned()
}

fn parse_effort(value: &str) -> Effort {
    match value.trim().to_ascii_lowercase().as_str() {
        "low" | "min" | "minimal" | "light" => Effort::Low,
        "high" => Effort::High,
        "xhigh" | "x-high" | "extra" | "very-high" | "very high" | "ultra" | "max" | "maximum" => {
            Effort::Xhigh
        }
        _ => Effort::Medium,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn request() -> DelegationRequest {
        DelegationRequest {
            schema_version: SCHEMA_VERSION,
            role: WorkerRole::Implementation,
            objective: "Add refresh-token rotation".into(),
            acceptance_criteria: vec![
                "Old refresh tokens become invalid".into(),
                "Existing auth tests remain green".into(),
            ],
            known_facts: vec!["Auth data is stored in SQLite".into()],
            decisions: vec!["Use the existing token store".into()],
            evidence_ids: Vec::new(),
            relevant_files: vec!["src/auth/store.rs".into()],
            owned_paths: vec!["src/auth/**".into()],
            write_mode: WriteMode::Isolated,
            capability_tier: CapabilityTier::Standard,
            effort: Effort::High,
            network_access: false,
            writable_output_paths: vec![],
            verification: vec!["cargo test auth".into()],
            output_contract: OutputContract::ImplementationResult,
            harness: Some("claude".into()),
            model: Some("fable".into()),
        }
    }

    fn result(status: WorkerResultStatus) -> WorkerResult {
        WorkerResult {
            schema_version: SCHEMA_VERSION,
            status,
            summary: "Completed the assigned work".into(),
            files_changed: vec!["src/auth/store.rs".into()],
            tests: vec![WorkerTestResult {
                command: "cargo test auth".into(),
                status: TestStatus::Passed,
                detail: Some("12 tests passed".into()),
            }],
            decisions: vec!["Kept the existing schema".into()],
            risks: Vec::new(),
            remaining_work: Vec::new(),
            suggested_next_action: SuggestedNextAction::Finish,
            suggested_role: (status == WorkerResultStatus::NeedsDelegation)
                .then_some(WorkerRole::Verification),
            suggested_task: (status == WorkerResultStatus::NeedsDelegation)
                .then(|| "Run the authentication regression suite".into()),
        }
    }

    fn result_block(result: &WorkerResult) -> String {
        format!(
            "```bridge-worker-result\n{}\n```",
            serde_json::to_string(result).unwrap()
        )
    }

    #[test]
    fn typed_request_round_trips_all_fields_and_schema_version() {
        let expected = request();
        let encoded = serde_json::to_string(&expected).unwrap();
        let text = format!("Plan:\n```bridge-delegate\n{encoded}\n```");
        let ParseOutcome::Parsed(NormalizedRequests { requests, .. }) = parse_delegation_requests(&text) else {
            panic!("typed request did not parse");
        };
        assert_eq!(requests, vec![expected.clone()]);
        assert_eq!(requests[0].schema_version, 1);
        assert_eq!(requests[0].runtime_harness(), "claude");
        assert_eq!(requests[0].model.as_deref(), Some("fable"));
        assert!(requests[0].validate().is_ok());
    }

    #[test]
    fn writable_output_paths_are_relative_and_cannot_escape() {
        let mut value = request();
        value.writable_output_paths = vec!["reports/result.json".into()];
        assert!(value.validate().is_ok());

        for invalid in [
            "",
            "/tmp/result",
            "../result",
            "reports/../result",
            "reports//result",
        ] {
            value.writable_output_paths = vec![invalid.into()];
            assert!(
                value.validate().is_err(),
                "accepted invalid output path: {invalid}"
            );
        }
    }

    #[test]
    fn typed_worker_result_round_trips_every_status() {
        let statuses = [
            WorkerResultStatus::Completed,
            WorkerResultStatus::Failed,
            WorkerResultStatus::Cancelled,
            WorkerResultStatus::Blocked,
            WorkerResultStatus::NeedsDelegation,
        ];
        for status in statuses {
            let expected = result(status);
            let ParseOutcome::Parsed(actual) = parse_worker_result(&result_block(&expected)) else {
                panic!("{status:?} did not parse");
            };
            assert_eq!(actual, expected);
        }
        let invalid = result_block(&result(WorkerResultStatus::Completed))
            .replace("\"completed\"", "\"invented_status\"");
        assert!(matches!(
            parse_worker_result(&invalid),
            ParseOutcome::Invalid { .. }
        ));
    }

    #[test]
    fn cancelled_is_terminal_and_not_retryable() {
        let cancelled = result(WorkerResultStatus::Cancelled);
        assert!(cancelled.is_terminal_cancellation());
        assert!(!cancelled.is_retryable());
        assert!(result(WorkerResultStatus::Failed).is_retryable());
        assert!(!result(WorkerResultStatus::Blocked).is_retryable());
    }

    #[test]
    fn needs_delegation_asks_for_a_task_not_a_vocabulary_word() {
        let mut needs = result(WorkerResultStatus::NeedsDelegation);
        assert!(needs.validate().is_ok());
        // Previously this was an error. A worker that had done real work should
        // not lose it for failing to guess a closed enum value; Bridge derives
        // the role instead.
        needs.suggested_role = None;
        assert!(needs.validate().is_ok(), "suggestedRole is derived, not demanded");
        // What it needs done is the one thing Bridge cannot infer.
        needs.suggested_task = Some(" ".into());
        assert!(needs.validate().unwrap_err().contains("suggestedTask"));
    }

    #[test]
    fn an_escalation_without_a_role_gets_one_derived_from_what_it_asked_for() {
        for (task, expected) in [
            ("verify the migration under load", WorkerRole::Verification),
            ("research how the provider paginates", WorkerRole::Research),
            ("document the new flag", WorkerRole::Documentation),
            ("decide between the two schemas", WorkerRole::Planning),
            ("wire the retry into the client", WorkerRole::Implementation),
        ] {
            let text = format!(
                "```bridge-worker-result\n{{\"status\":\"escalate\",\"summary\":\"needs a specialist\",\"suggestedTask\":\"{task}\"}}\n```"
            );
            let ParseOutcome::Parsed(parsed) = parse_worker_result(&text) else {
                panic!("escalation without a role did not parse: {task}");
            };
            assert_eq!(parsed.status, WorkerResultStatus::NeedsDelegation);
            assert_eq!(parsed.suggested_role, Some(expected), "{task}");
            assert_eq!(parsed.suggested_next_action, SuggestedNextAction::FollowUp);
        }
    }

    #[test]
    fn legacy_fenced_directive_converts_at_parse_boundary() {
        let text = r#"```bridge-delegate
{"harness":"anthropic","model":"fable","effort":"ultra","task":"Refactor auth","context":"Keep the public API stable"}
```"#;
        let ParseOutcome::Parsed(NormalizedRequests { requests, .. }) = parse_delegation_requests(text) else {
            panic!("legacy directive did not parse");
        };
        assert_eq!(requests.len(), 1);
        let typed = &requests[0];
        assert_eq!(typed.schema_version, SCHEMA_VERSION);
        assert_eq!(typed.role, WorkerRole::Implementation);
        assert_eq!(typed.objective, "Refactor auth");
        assert_eq!(typed.known_facts, vec!["Keep the public API stable"]);
        assert_eq!(typed.write_mode, WriteMode::Shared);
        assert_eq!(typed.capability_tier, CapabilityTier::Standard);
        assert_eq!(typed.effort, Effort::Xhigh);
        assert_eq!(typed.runtime_harness(), "claude");
        assert_eq!(typed.model.as_deref(), Some("fable"));
    }

    #[test]
    fn harness_hints_fold_aliases_and_admit_installed_agents() {
        for (hint, expected) in [
            ("claude-code", "claude"),
            ("Anthropic", "claude"),
            ("openai", "codex"),
            ("open-code", "opencode"),
            // Any installed agent is delegable by its own id — that is what
            // puts the marketplace inside the delegation tree.
            ("gemini", "gemini"),
            ("  Gemini  ", "gemini"),
            ("github-copilot-cli", "github-copilot-cli"),
        ] {
            assert_eq!(
                normalize_harness(hint).as_deref(),
                Some(expected),
                "hint {hint:?}"
            );
        }
    }

    #[test]
    fn an_unrecognized_harness_hint_is_refused_rather_than_defaulted() {
        // `runtime_harness()` ends in `unwrap_or("codex")`, so a hint that
        // normalizes to None must be rejected by `validate()` rather than
        // silently becoming a Codex worker.
        for hint in ["", "acp:gemini", "gpt 9", "-gemini", "gem/ini"] {
            assert_eq!(normalize_harness(hint), None, "hint {hint:?}");
        }
    }

    #[test]
    fn a_delegation_directive_cannot_summon_a_shell_worker() {
        // `shell` is a valid harness id but deliberately absent from the hint
        // table: opening the identifier must not widen what a model is allowed
        // to delegate to.
        assert_eq!(normalize_harness("shell"), None);
        assert_eq!(normalize_harness("Shell"), None);
        assert_eq!(normalize_harness("  shell  "), None);
        // ...while every other agent stays reachable.
        assert_eq!(normalize_harness("goose").as_deref(), Some("goose"));
    }

    #[test]
    fn malformed_output_requests_one_same_session_repair_then_unstructured_fallback() {
        let mut tracker = ResultRepairTracker::default();
        let sends = Cell::new(0);
        let first = tracker.process("worker-1", "not json", |prompt| {
            sends.set(sends.get() + 1);
            assert!(prompt.contains("one repair turn"));
            assert!(prompt.contains("bridge-worker-result"));
            true
        });
        assert!(matches!(first, WorkerOutputAction::AwaitingRepair { .. }));
        let second = tracker.process("worker-1", "still not json", |_| {
            panic!("a second repair turn must never be sent")
        });
        let WorkerOutputAction::Unstructured { raw, reason } = second else {
            panic!("second failure was not labeled unstructured");
        };
        assert!(raw.contains("Initial invalid output:\nnot json"));
        assert!(raw.contains("Invalid repair output:\nstill not json"));
        assert!(reason.contains("missing bridge-worker-result"));
        assert_eq!(sends.get(), 1);
    }

    #[test]
    fn valid_repair_clears_repair_state() {
        let mut tracker = ResultRepairTracker::default();
        assert!(matches!(
            tracker.process("worker", "bad", |_| true),
            WorkerOutputAction::AwaitingRepair { .. }
        ));
        let corrected = result(WorkerResultStatus::Completed);
        assert_eq!(
            tracker.process("worker", &result_block(&corrected), |_| false),
            WorkerOutputAction::Structured(corrected)
        );
        assert!(matches!(
            tracker.process("worker", "new bad output", |_| true),
            WorkerOutputAction::AwaitingRepair { .. }
        ));
    }

    #[test]
    fn failed_repair_delivery_falls_back_without_spawning() {
        let mut tracker = ResultRepairTracker::default();
        let action = tracker.process("same-worker", "bad output", |prompt| {
            assert!(prompt.contains("Do not perform more work"));
            false
        });
        let WorkerOutputAction::Unstructured { raw, reason } = action else {
            panic!("undeliverable repair did not fall back");
        };
        assert_eq!(raw, "bad output");
        assert!(reason.contains("same-session repair could not be delivered"));
    }

    #[test]
    fn strip_directives_removes_machine_blocks_only() {
        let typed = serde_json::to_string(&request()).unwrap();
        let text = format!(
            "Prose before.\n```rust\nlet x = 1;\n```\n```bridge-delegate\n{typed}\n```\nProse after."
        );
        let stripped = strip_directives(&text);
        assert!(stripped.contains("Prose before."));
        assert!(stripped.contains("```rust"));
        assert!(stripped.contains("let x = 1;"));
        assert!(stripped.contains("Prose after."));
        assert!(!stripped.contains("bridge-delegate"));
        assert!(!stripped.contains("acceptanceCriteria"));

        let result_text = format!(
            "Visible.\n{}",
            result_block(&result(WorkerResultStatus::Completed))
        );
        assert_eq!(strip_worker_result(&result_text), "Visible.");
    }

    #[test]
    fn a_depth_one_worker_may_ask_for_one_specialist_without_spawning_it() {
        assert_eq!(DEFAULT_MAX_DEPTH, 1);
        assert!(protocol(0).contains("Default topology is flat"));
        assert!(protocol(0).contains("schemaVersion"));
        // Still cannot spawn: the depth limit is unchanged and stays in Rust.
        assert!(protocol(1).contains("you do not spawn workers yourself"));
        assert!(!protocol(1).contains("```bridge-delegate"));
        // But it is told how to ask, and told that asking costs it nothing.
        assert!(protocol(1).contains("needs_delegation"));
        assert!(protocol(1).contains("`suggestedRole` is optional"));
        assert!(protocol(1).contains("never means discarding your own findings"));
        assert!(protocol(1).contains("policy gate"));
    }

    #[test]
    fn worker_briefing_contains_typed_output_contract() {
        let result = result(WorkerResultStatus::Completed);
        let evidence = WorkerEvidence {
            evidence_id: "entry-evidence-1".into(),
            child_session_id: "worker-1".into(),
            result: result.clone(),
        };
        let briefing = worker_briefing(
            &request(),
            1,
            "bridge/auth-kyoto",
            std::slice::from_ref(&evidence),
        );
        assert!(briefing.contains("Add refresh-token rotation"));
        assert!(briefing.contains("Old refresh tokens become invalid"));
        assert!(briefing.contains("Auth data is stored in SQLite"));
        assert!(briefing.contains("src/auth/store.rs"));
        assert!(briefing.contains("cargo test auth"));
        assert!(briefing.contains("bridge-worker-result"));
        assert!(briefing.contains("schemaVersion"));
        assert!(briefing.contains("you need one focused specialist"));
        assert!(briefing.contains("```mermaid"));
        assert!(briefing.contains("sandboxed iframe"));
        assert!(briefing.contains("entry-evidence-1"));
        assert!(briefing.contains("worker-1"));
        assert!(briefing.contains(&serde_json::to_string(&result).unwrap()));
    }

    #[test]
    fn evidence_ids_are_bounded_unique_and_ordered() {
        let mut request = request();
        request.evidence_ids = vec!["evidence-2".into(), "evidence-1".into()];
        request.validate().unwrap();
        assert_eq!(request.evidence_ids, ["evidence-2", "evidence-1"]);
        request.evidence_ids.push("evidence-2".into());
        assert!(request.validate().unwrap_err().contains("duplicates"));
        request.evidence_ids = (0..=MAX_EVIDENCE_REFERENCES)
            .map(|index| format!("evidence-{index}"))
            .collect();
        assert!(request.validate().unwrap_err().contains("more than"));
    }

    #[test]
    fn malformed_or_mixed_request_blocks_are_rejected_atomically() {
        assert!(matches!(
            parse_delegation_requests("```bridge-delegate\nnot json\n```"),
            ParseOutcome::Invalid { .. }
        ));
        let valid = serde_json::to_string(&request()).unwrap();
        // One unusable member still rejects the whole block: partially
        // dispatching a batch would leave the orchestrator guessing which
        // workers exist. An empty objective is the case Bridge cannot fill in.
        let mixed = format!(
            "```bridge-delegate\n[{valid},{{\"objective\":\"   \"}}]\n```"
        );
        assert!(matches!(
            parse_delegation_requests(&mixed),
            ParseOutcome::Invalid { .. }
        ));
        // A member missing only transport fields is not unusable — Bridge owns
        // those, so the batch goes through.
        let sparse = format!(
            "```bridge-delegate\n[{valid},{{\"objective\":\"Add a regression test\",\"role\":\"verification\"}}]\n```"
        );
        let ParseOutcome::Parsed(NormalizedRequests { requests, .. }) = parse_delegation_requests(&sparse) else {
            panic!("a request carrying only semantics was rejected");
        };
        assert_eq!(requests.len(), 2);
        assert_eq!(
            parse_delegation_requests("ordinary prose"),
            ParseOutcome::Absent
        );
    }

    #[test]
    fn write_mode_none_is_read_as_read_only_rather_than_failing_the_request() {
        // `writeMode:"none"` is what a model naturally writes for a read-only
        // role. It used to fail the request, cost a correction turn, and start
        // no worker. It means readOnly; Bridge reads it that way.
        let request = r#"```bridge-delegate
{"role":"research","objective":"map the delegation tree","writeMode":"none"}
```"#;
        let ParseOutcome::Parsed(NormalizedRequests { requests, .. }) = parse_delegation_requests(request) else {
            panic!("writeMode:none was still rejected");
        };
        assert_eq!(requests[0].write_mode, WriteMode::ReadOnly);
        assert_eq!(requests[0].output_contract, OutputContract::ResearchResult);
    }

    #[test]
    fn a_read_only_role_cannot_buy_write_access_with_a_generous_wire_format() {
        // The point of the clamp. Opening the format must not open the
        // permissions: whatever the model asks for, a research worker reads.
        for asked in ["full", "shared", "isolated", "danger-full-access"] {
            let request = format!(
                "```bridge-delegate\n{{\"role\":\"researcher\",\"objective\":\"read the store\",\"writeMode\":\"{asked}\"}}\n```"
            );
            let ParseOutcome::Parsed(NormalizedRequests { requests, .. }) = parse_delegation_requests(&request) else {
                panic!("{asked} did not parse");
            };
            assert_eq!(
                requests[0].write_mode,
                WriteMode::ReadOnly,
                "a research worker asked for {asked} and must still be read-only"
            );
        }
        // An implementation worker keeps the write mode it is entitled to.
        let request = r#"```bridge-delegate
{"role":"implementer","objective":"add the retry"}
```"#;
        let ParseOutcome::Parsed(NormalizedRequests { requests, .. }) = parse_delegation_requests(request) else {
            panic!("implementation request did not parse");
        };
        assert_eq!(requests[0].role, WorkerRole::Implementation);
        assert_eq!(requests[0].write_mode, WriteMode::Isolated);
    }

    #[test]
    fn a_request_carrying_only_semantics_gets_its_transport_from_rust() {
        // The whole authority argument in one assertion: the model wrote an
        // objective and a role, and Bridge produced a complete, valid request.
        let request = r#"```bridge-delegate
{"role":"implementation","objective":"Add refresh-token rotation"}
```"#;
        let ParseOutcome::Parsed(NormalizedRequests { requests, .. }) = parse_delegation_requests(request) else {
            panic!("a semantics-only request was rejected");
        };
        let parsed = &requests[0];
        assert_eq!(parsed.schema_version, SCHEMA_VERSION);
        assert_eq!(parsed.write_mode, WriteMode::Isolated);
        assert_eq!(parsed.capability_tier, CapabilityTier::Standard);
        assert_eq!(parsed.effort, Effort::Medium);
        assert_eq!(parsed.output_contract, OutputContract::ImplementationResult);
        assert_eq!(parsed.acceptance_criteria.len(), 1);
        assert!(parsed.known_facts.is_empty());
        assert!(parsed.owned_paths.is_empty());
        parsed.validate().expect("the derived request is valid");
    }

    #[test]
    fn role_spellings_a_model_actually_writes_all_land_somewhere() {
        for (written, expected) in [
            ("implementer", WorkerRole::Implementation),
            ("coder", WorkerRole::Implementation),
            ("implementation-verifier", WorkerRole::Verification),
            ("reviewer", WorkerRole::Verification),
            ("QA", WorkerRole::Verification),
            ("researcher", WorkerRole::Research),
            ("orchestrator", WorkerRole::Planning),
            ("architect", WorkerRole::Planning),
            ("docs", WorkerRole::Documentation),
            ("Technical Writer", WorkerRole::Documentation),
        ] {
            assert_eq!(WorkerRole::parse(written), Some(expected), "{written}");
        }
        // Widened, not opened: a role Bridge does not have is still no role.
        assert_eq!(WorkerRole::parse("security-auditor"), None);
        assert_eq!(WorkerRole::parse(""), None);
    }

    #[test]
    fn result_vocabulary_a_model_actually_writes_all_lands_somewhere() {
        for (written, expected) in [
            ("success", WorkerResultStatus::Completed),
            ("done", WorkerResultStatus::Completed),
            ("error", WorkerResultStatus::Failed),
            ("escalate", WorkerResultStatus::NeedsDelegation),
            ("needs-delegation", WorkerResultStatus::NeedsDelegation),
            ("aborted", WorkerResultStatus::Cancelled),
            ("stuck", WorkerResultStatus::Blocked),
        ] {
            assert_eq!(WorkerResultStatus::parse(written), Some(expected), "{written}");
        }
        for (written, expected) in [
            ("delegate_implementation", SuggestedNextAction::FollowUp),
            ("escalate", SuggestedNextAction::FollowUp),
            ("continue", SuggestedNextAction::FollowUp),
            ("done", SuggestedNextAction::Finish),
            ("ask_user", SuggestedNextAction::RequestApproval),
        ] {
            assert_eq!(SuggestedNextAction::parse(written), Some(expected), "{written}");
        }
        // An unreadable suggestion falls back to one derived from the status
        // rather than failing a result that reported real work.
        let text = r#"```bridge-worker-result
{"status":"success","summary":"tests pass","suggestedNextAction":"celebrate"}
```"#;
        let ParseOutcome::Parsed(parsed) = parse_worker_result(text) else {
            panic!("an unreadable suggestion failed the result");
        };
        assert_eq!(parsed.status, WorkerResultStatus::Completed);
        assert_eq!(parsed.suggested_next_action, SuggestedNextAction::Finish);
    }

    #[test]
    fn unreadable_output_is_protocol_invalid_and_keeps_the_workers_own_words() {
        let prose = "I refactored the token store and cargo test auth is green, but I forgot the fence.";
        let result = protocol_invalid_result(prose, "missing bridge-worker-result block");
        assert_eq!(result.status, WorkerResultStatus::ProtocolInvalid);
        assert!(result.is_protocol_invalid());
        // The two properties that matter: not a task failure, and not a retry.
        assert!(
            !result.is_retryable(),
            "a formatting mistake must not buy another paid turn"
        );
        // And the work is not thrown away just because the envelope was wrong.
        assert!(result.summary.contains("cargo test auth is green"));
        assert!(result.summary.contains("could not be read"));
        assert!(result.risks.iter().any(|risk| risk.contains("nothing here has been verified")));
        result.validate().expect("the preserved result is still valid");

        // A worker that said nothing at all is described honestly too.
        let empty = protocol_invalid_result("", "worker finished without a text summary");
        assert_eq!(empty.status, WorkerResultStatus::ProtocolInvalid);
        assert!(empty.summary.contains("left no prose"));
        empty.validate().unwrap();
    }

    #[test]
    fn preserved_prose_is_bounded() {
        let huge = "x".repeat(MAX_PRESERVED_PROSE_BYTES * 3);
        let result = protocol_invalid_result(&huge, "invalid worker-result JSON");
        assert!(
            result.summary.len() < MAX_PRESERVED_PROSE_BYTES + 300,
            "a parent's context is not a place to paste a transcript"
        );
        assert!(result.summary.contains('…'));
    }

    /// The exact envelope shapes the production database recorded as failures.
    ///
    /// Every one of these cost a worker, a correction turn, or both. The
    /// assertion is that none of them is a task failure any more: each either
    /// parses into usable work, or is classified `protocol_invalid` — and
    /// neither outcome is retryable.
    #[test]
    fn the_historical_failure_corpus_no_longer_reads_as_task_failure() {
        let requests = [
            // invalid suggestedRole vocabulary, mirrored on the request side
            r#"{"role":"implementer","objective":"Fix the migration guard"}"#,
            // unknown field
            r#"{"role":"research","objective":"Find the stall","confidence":"medium"}"#,
            // writeMode a read-only role should never have been asked to author
            r#"{"role":"verification","objective":"Re-run the suite","writeMode":"none"}"#,
            // a guessed schema version
            r#"{"schemaVersion":2,"role":"planning","objective":"Choose a store"}"#,
        ];
        for body in requests {
            let text = format!("```bridge-delegate\n{body}\n```");
            let ParseOutcome::Parsed(parsed) = parse_delegation_requests(&text) else {
                panic!("historical request still rejected: {body}");
            };
            parsed.requests[0].validate().expect(body);
            // And what Bridge did on the model's behalf is on the record, not
            // applied invisibly.
            assert!(
                !parsed.notes.is_empty(),
                "a rescued request should say what was rescued: {body}"
            );
        }

        let results = [
            // invalid suggestedRole
            r#"{"status":"needs_delegation","summary":"needs a specialist","suggestedRole":"implementation-verifier","suggestedTask":"verify the fix"}"#,
            // invalid suggestedNextAction
            r#"{"status":"completed","summary":"done","suggestedNextAction":"delegate_implementation"}"#,
            // unknown field
            r#"{"status":"completed","summary":"done","tokensUsed":1234}"#,
            // a status in the model's own words
            r#"{"status":"success","summary":"all green"}"#,
        ];
        for body in results {
            let text = format!("```bridge-worker-result\n{body}\n```");
            let ParseOutcome::Parsed(parsed) = parse_worker_result(&text) else {
                panic!("historical result still rejected: {body}");
            };
            assert!(
                !parsed.is_protocol_invalid(),
                "this one is usable, not transport noise: {body}"
            );
            assert!(
                !parsed.is_retryable(),
                "a usable result must not look retryable: {body}"
            );
        }

        // The one shape normalization genuinely cannot rescue: no fence at all.
        // It is reported as transport, not as a failed task, and not retried.
        let unfenced = "I finished the work but wrote no result block.";
        assert_eq!(parse_worker_result(unfenced), ParseOutcome::Absent);
        let classified = protocol_invalid_result(unfenced, "missing bridge-worker-result block");
        assert_eq!(classified.status, WorkerResultStatus::ProtocolInvalid);
        assert!(!classified.is_retryable());
    }

    #[test]
    fn correction_feedback_still_names_what_a_request_must_carry() {
        // Genuinely unusable requests still get feedback, and it still says no
        // worker ran — the correction path did not disappear, it got rarer.
        let feedback = invalid_request_feedback("objective must not be empty");
        assert!(feedback.contains("objective must not be empty"));
        assert!(feedback.contains("no worker started"));
        assert!(feedback.contains("objective: what the worker must accomplish"));
        // And it stops teaching the model to author fields Bridge owns.
        assert!(!feedback.contains("capabilityTier: fast"));
        assert!(feedback.contains("Bridge supplies the schema version"));
    }

    #[test]
    fn unclosed_machine_fence_preserves_text() {
        let text = "Before\n```bridge-delegate\n{not valid}\nImportant prose after";
        assert_eq!(strip_directives(text), text);
        let result = "Before\n```bridge-worker-result\n{not valid}\nImportant prose after";
        assert_eq!(strip_worker_result(result), result);
    }

    #[test]
    fn a_wrong_schema_version_and_an_extra_field_are_transport_noise_not_failures() {
        // Both of these used to fail the envelope. There is one schema version
        // and Bridge owns it, so a model guessing `2` is a spelling mistake in
        // a field it should never have had to write; an extra explanatory key is
        // the model being helpful. Neither is a reason to discard the work.
        let request = r#"```bridge-delegate
{"schemaVersion":2,"objective":"future","futureField":true,"why":"explaining myself"}
```"#;
        let ParseOutcome::Parsed(NormalizedRequests { requests, .. }) = parse_delegation_requests(request) else {
            panic!("a request with transport noise was rejected");
        };
        assert_eq!(requests[0].schema_version, SCHEMA_VERSION);
        assert_eq!(requests[0].objective, "future");

        let result = r#"```bridge-worker-result
{"schemaVersion":2,"status":"completed","summary":"did the thing","confidence":"high"}
```"#;
        let ParseOutcome::Parsed(parsed) = parse_worker_result(result) else {
            panic!("a result with transport noise was rejected");
        };
        assert_eq!(parsed.schema_version, SCHEMA_VERSION);
        assert_eq!(parsed.status, WorkerResultStatus::Completed);
        assert_eq!(parsed.summary, "did the thing");
    }
}

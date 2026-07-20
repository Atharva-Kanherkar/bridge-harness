//! Typed delegation transport and worker-result protocol.
//!
//! Harness messages may still carry fenced `bridge-delegate` JSON, but parsing
//! immediately produces [`DelegationRequest`]. No free-form task/context object
//! crosses that boundary. Workers return a versioned [`WorkerResult`].

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
}

impl WorkerResultStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Blocked => "blocked",
            Self::NeedsDelegation => "needs_delegation",
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
            if self.suggested_role.is_none() {
                return Err("needs_delegation requires suggestedRole".into());
            }
            require_non_empty(
                "suggestedTask",
                self.suggested_task.as_deref().unwrap_or_default(),
            )?;
        }
        Ok(())
    }

    pub fn is_retryable(&self) -> bool {
        self.status == WorkerResultStatus::Failed
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

pub fn parse_delegation_requests(text: &str) -> ParseOutcome<Vec<DelegationRequest>> {
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
                Ok(request) => requests.push(request),
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
        ParseOutcome::Parsed(requests)
    }
}

fn request_from_value(value: Value) -> Result<DelegationRequest, String> {
    if value.get("schemaVersion").is_some() || value.get("objective").is_some() {
        validate_schema_version(&value, "delegation")?;
        let request: DelegationRequest =
            serde_json::from_value(value).map_err(|error| error.to_string())?;
        request.validate()?;
        Ok(request)
    } else {
        serde_json::from_value::<LegacyDirective>(value)
            .map_err(|error| error.to_string())?
            .into_typed()
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
    let value = match serde_json::from_str::<Value>(&raw) {
        Ok(value) => value,
        Err(error) => {
            return ParseOutcome::Invalid {
                raw,
                reason: format!("invalid worker-result JSON: {error}"),
            }
        }
    };
    if let Err(reason) = validate_schema_version(&value, "worker-result") {
        return ParseOutcome::Invalid { raw, reason };
    }
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

fn validate_schema_version(value: &Value, envelope: &str) -> Result<(), String> {
    let Some(version) = value.get("schemaVersion").and_then(Value::as_u64) else {
        return Err(format!("{envelope} schemaVersion must be an integer"));
    };
    if version != SCHEMA_VERSION as u64 {
        return Err(format!(
            "unsupported {envelope} schema version {version}; expected {SCHEMA_VERSION}"
        ));
    }
    Ok(())
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

pub fn worker_result_repair_prompt(reason: &str) -> String {
    format!(
        r#"Your previous final output could not be parsed ({reason}). This is your one repair turn. Do not perform more work. Return exactly one fenced `bridge-worker-result` JSON object matching schemaVersion 1 with: status, summary, filesChanged, tests, decisions, risks, remainingWork, suggestedNextAction, and optional suggestedRole/suggestedTask. Do not add prose outside the fence."#
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
        return r#"## Bridge worker topology

You are a depth-one worker. Do not spawn or directly delegate to another worker. If more specialization is required, return a typed worker result with `status: "needs_delegation"`, `suggestedRole`, and `suggestedTask`; the parent and Rust policy gate decide what happens next."#
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

pub fn worker_briefing(
    request: &DelegationRequest,
    depth: i64,
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
        r#"You are a Bridge {role:?} worker assigned one focused objective on branch `{branch}`.

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
{verification}

Complete only this objective. Do not directly delegate. If blocked on another specialist, return `needs_delegation` to the parent.

End with exactly one fenced `bridge-worker-result` JSON object matching schemaVersion 1:

```bridge-worker-result
{{"schemaVersion":1,"status":"completed","summary":"What changed or was found","filesChanged":[],"tests":[{{"command":"command run","status":"passed"}}],"decisions":[],"risks":[],"remainingWork":[],"suggestedNextAction":"finish"}}
```

{protocol}"#,
        role = request.role,
        objective = request.objective,
        write_mode = request.write_mode,
        tier = request.capability_tier,
        effort = request.effort.as_str(),
        protocol = protocol(depth),
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

pub fn normalize_harness(value: &str) -> Option<String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "claude" | "claude-code" | "claudecode" | "anthropic" => Some("claude".into()),
        "codex" | "gpt" | "openai" => Some("codex".into()),
        "opencode" | "open-code" => Some("opencode".into()),
        _ => None,
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
        let ParseOutcome::Parsed(requests) = parse_delegation_requests(&text) else {
            panic!("typed request did not parse");
        };
        assert_eq!(requests, vec![expected.clone()]);
        assert_eq!(requests[0].schema_version, 1);
        assert_eq!(requests[0].runtime_harness(), "claude");
        assert_eq!(requests[0].model.as_deref(), Some("fable"));
        assert!(requests[0].validate().is_ok());
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
    fn needs_delegation_requires_suggestion() {
        let mut needs = result(WorkerResultStatus::NeedsDelegation);
        assert!(needs.validate().is_ok());
        needs.suggested_role = None;
        assert!(needs.validate().unwrap_err().contains("suggestedRole"));
        needs.suggested_role = Some(WorkerRole::Verification);
        needs.suggested_task = Some(" ".into());
        assert!(needs.validate().unwrap_err().contains("suggestedTask"));
    }

    #[test]
    fn legacy_fenced_directive_converts_at_parse_boundary() {
        let text = r#"```bridge-delegate
{"harness":"anthropic","model":"fable","effort":"ultra","task":"Refactor auth","context":"Keep the public API stable"}
```"#;
        let ParseOutcome::Parsed(requests) = parse_delegation_requests(text) else {
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
    fn flat_protocol_forbids_worker_delegation() {
        assert_eq!(DEFAULT_MAX_DEPTH, 1);
        assert!(protocol(0).contains("Default topology is flat"));
        assert!(protocol(0).contains("schemaVersion"));
        assert!(protocol(1).contains("Do not spawn or directly delegate"));
        assert!(protocol(1).contains("needs_delegation"));
        assert!(!protocol(1).contains("```bridge-delegate"));
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
        assert!(briefing.contains("Do not directly delegate"));
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
        let mixed = format!(
            "```bridge-delegate\n[{valid},{{\"schemaVersion\":1,\"objective\":\"missing fields\"}}]\n```"
        );
        assert!(matches!(
            parse_delegation_requests(&mixed),
            ParseOutcome::Invalid { .. }
        ));
        assert_eq!(
            parse_delegation_requests("ordinary prose"),
            ParseOutcome::Absent
        );
    }

    #[test]
    fn unclosed_machine_fence_preserves_text() {
        let text = "Before\n```bridge-delegate\n{not valid}\nImportant prose after";
        assert_eq!(strip_directives(text), text);
        let result = "Before\n```bridge-worker-result\n{not valid}\nImportant prose after";
        assert_eq!(strip_worker_result(result), result);
    }

    #[test]
    fn unsupported_schema_version_precedes_unknown_field_error() {
        let request = r#"```bridge-delegate
{"schemaVersion":2,"objective":"future","futureField":true}
```"#;
        let ParseOutcome::Invalid { reason, .. } = parse_delegation_requests(request) else {
            panic!("future request was not rejected");
        };
        assert!(reason.contains("unsupported delegation schema version 2"));

        let result = r#"```bridge-worker-result
{"schemaVersion":2,"status":"completed","futureField":true}
```"#;
        let ParseOutcome::Invalid { reason, .. } = parse_worker_result(result) else {
            panic!("future result was not rejected");
        };
        assert!(reason.contains("unsupported worker-result schema version 2"));
    }
}

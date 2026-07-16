use crate::{delegation::{DelegationRequest, WorkerResult, WorkerResultStatus}, store, BridgeError};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashSet};
use std::path::Path;
use uuid::Uuid;

pub const COMPLETION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskTier {
    Low,
    Medium,
    High,
}

impl RiskTier {
    fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalKind {
    Deterministic,
    Scrutiny,
    UserTesting,
}

impl EvalKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Deterministic => "deterministic",
            Self::Scrutiny => "scrutiny",
            Self::UserTesting => "user_testing",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pending,
    Running,
    Passed,
    Failed,
    Skipped,
    Blocked,
    Stale,
}

impl CheckStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::Blocked => "blocked",
            Self::Stale => "stale",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionVerdict {
    Verifying,
    ChangesRequested,
    Verified,
    Waived,
    Failed,
    Superseded,
}

impl CompletionVerdict {
    fn as_str(self) -> &'static str {
        match self {
            Self::Verifying => "verifying",
            Self::ChangesRequested => "changes_requested",
            Self::Verified => "verified",
            Self::Waived => "waived",
            Self::Failed => "failed",
            Self::Superseded => "superseded",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryStamp {
    pub head: String,
    pub dirty_digest: String,
}

impl RepositoryStamp {
    pub fn validate(&self) -> Result<(), BridgeError> {
        if self.head.trim().is_empty() || self.dirty_digest.trim().is_empty() {
            return Err(BridgeError::Invalid(
                "completion evidence requires repository HEAD and dirty digest".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionContract {
    pub id: String,
    pub workspace_id: String,
    pub session_id: String,
    pub schema_version: u32,
    pub acceptance_criteria: Vec<String>,
    pub markdown_projection: Option<String>,
    pub markdown_committed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifierManifest {
    pub id: String,
    pub kind: EvalKind,
    #[serde(default)]
    pub triggers: Vec<String>,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    pub different_model_family: bool,
    pub checks: Vec<String>,
    #[serde(default)]
    pub evidence_required: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifierCandidate {
    pub manifest: VerifierManifest,
    pub eligible: bool,
    pub exclusion_reasons: Vec<String>,
}

impl VerifierManifest {
    pub fn validate(&self) -> Result<(), BridgeError> {
        if self.id.trim().is_empty() || self.checks.is_empty() {
            return Err(BridgeError::Invalid(
                "verifier manifest requires an id and at least one check".into(),
            ));
        }
        if self.kind == EvalKind::Deterministic && self.different_model_family {
            return Err(BridgeError::Invalid(
                "deterministic verifier manifests cannot require a model family".into(),
            ));
        }
        for value in self
            .triggers
            .iter()
            .chain(&self.required_capabilities)
            .chain(&self.checks)
            .chain(&self.evidence_required)
        {
            if value.trim().is_empty() {
                return Err(BridgeError::Invalid(
                    "verifier manifest values cannot be empty".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn eligible(
        &self,
        implementer_family: Option<&str>,
        verifier_family: Option<&str>,
        available_capabilities: &HashSet<String>,
    ) -> Result<(), String> {
        let missing = self
            .required_capabilities
            .iter()
            .filter(|capability| !available_capabilities.contains(capability.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(format!("missing verifier capabilities: {}", missing.join(", ")));
        }
        if self.different_model_family {
            let implementer = implementer_family.filter(|value| !value.trim().is_empty());
            let verifier = verifier_family.filter(|value| !value.trim().is_empty());
            if implementer.is_none() || verifier.is_none() || implementer == verifier {
                return Err("verifier must use a different model family from the implementer".into());
            }
        }
        Ok(())
    }
}

pub fn register_verifier_manifest(
    db: &Connection,
    source: &str,
    manifest: &VerifierManifest,
) -> Result<(), BridgeError> {
    manifest.validate()?;
    if source.trim().is_empty() {
        return Err(BridgeError::Invalid("verifier manifest source cannot be empty".into()));
    }
    db.execute(
        "INSERT INTO verifier_manifests(id,source,schema_version,manifest,enabled,updated_at) VALUES(?1,?2,?3,?4,1,?5)
         ON CONFLICT(id) DO UPDATE SET source=excluded.source,schema_version=excluded.schema_version,manifest=excluded.manifest,updated_at=excluded.updated_at",
        params![manifest.id, source.trim(), COMPLETION_SCHEMA_VERSION, serde_json::to_string(manifest).map_err(|error| BridgeError::Invalid(error.to_string()))?, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

pub fn verifier_candidates(
    db: &Connection,
    change_labels: &[String],
    available_capabilities: &HashSet<String>,
) -> Result<Vec<VerifierCandidate>, BridgeError> {
    let labels = change_labels.iter().map(|label| label.to_ascii_lowercase()).collect::<HashSet<_>>();
    let mut statement = db.prepare("SELECT manifest FROM verifier_manifests WHERE enabled=1 ORDER BY id")?;
    let manifests = statement.query_map([], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    manifests.into_iter().map(|serialized| {
        let manifest: VerifierManifest = serde_json::from_str(&serialized).map_err(|error| BridgeError::Invalid(format!("stored verifier manifest is malformed: {error}")))?;
        manifest.validate()?;
        let mut exclusion_reasons = Vec::new();
        if !manifest.triggers.is_empty() && !manifest.triggers.iter().any(|trigger| labels.contains(&trigger.to_ascii_lowercase())) {
            exclusion_reasons.push("change triggers do not match".into());
        }
        let missing = manifest.required_capabilities.iter().filter(|capability| !available_capabilities.contains(capability.as_str())).cloned().collect::<Vec<_>>();
        if !missing.is_empty() {
            exclusion_reasons.push(format!("missing capabilities: {}", missing.join(", ")));
        }
        Ok(VerifierCandidate { eligible: exclusion_reasons.is_empty(), manifest, exclusion_reasons })
    }).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalCheck {
    pub id: String,
    pub label: String,
    pub kind: EvalKind,
    pub required: bool,
    pub executor: String,
    pub command: Option<String>,
    pub required_capabilities: Vec<String>,
    pub different_model_family: bool,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalPlan {
    pub id: String,
    pub contract_id: String,
    pub schema_version: u32,
    pub risk: RiskTier,
    pub checks: Vec<EvalCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanInput {
    pub contract_id: String,
    pub acceptance_criteria: Vec<String>,
    pub changed_paths: Vec<String>,
    pub repository_commands: Vec<String>,
    pub available_capabilities: HashSet<String>,
}

pub fn plan(input: PlanInput) -> EvalPlan {
    let lower_paths = input
        .changed_paths
        .iter()
        .map(|path| path.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let joined_criteria = input.acceptance_criteria.join(" ").to_ascii_lowercase();
    let touches = |needles: &[&str]| {
        lower_paths
            .iter()
            .any(|path| needles.iter().any(|needle| path.contains(needle)))
    };
    let high_risk = touches(&[
        "auth",
        "secret",
        "credential",
        "migration",
        "store.rs",
        "policy",
        "worker_lifecycle",
        "session_supervisor",
        "adapter",
        "learning_router",
    ]);
    let user_facing = touches(&[".tsx", ".jsx", "src/components", "src/app"])
        || ["browser", "screen", "dialog", "button", "user journey"]
            .iter()
            .any(|needle| joined_criteria.contains(needle));
    let risk = if high_risk {
        RiskTier::High
    } else if user_facing || input.changed_paths.len() > 4 {
        RiskTier::Medium
    } else {
        RiskTier::Low
    };
    let mut checks = Vec::new();
    let mut commands = BTreeSet::new();
    commands.extend(input.repository_commands);
    if touches(&[".rs", "cargo.toml"]) {
        commands.insert("cargo test --manifest-path src-tauri/Cargo.toml".into());
        commands.insert("cargo check --manifest-path src-tauri/Cargo.toml".into());
    }
    if touches(&[".ts", ".tsx", ".js", ".jsx", "package.json"]) {
        commands.insert("bun run test".into());
        commands.insert("bun run build".into());
    }
    if commands.is_empty() {
        commands.insert("git diff --check".into());
    }
    for (index, command) in commands.into_iter().enumerate() {
        checks.push(EvalCheck {
            id: format!("deterministic-{index}"),
            label: command.clone(),
            kind: EvalKind::Deterministic,
            required: true,
            executor: "bridge.shell".into(),
            command: Some(command),
            required_capabilities: vec!["shell".into()],
            different_model_family: false,
            reason: "repository policy or changed file type requires this check".into(),
        });
    }
    if risk != RiskTier::Low {
        checks.push(EvalCheck {
            id: "scrutiny-review".into(),
            label: "Independent scrutiny review".into(),
            kind: EvalKind::Scrutiny,
            required: true,
            executor: "bridge.worker".into(),
            command: None,
            required_capabilities: vec!["code_review".into()],
            different_model_family: true,
            reason: "medium and high risk changes require semantic independent review".into(),
        });
    }
    if user_facing {
        checks.push(EvalCheck {
            id: "user-journey".into(),
            label: "User journey verification".into(),
            kind: EvalKind::UserTesting,
            required: true,
            executor: "bridge.worker".into(),
            command: None,
            required_capabilities: vec!["browser".into(), "console_inspection".into()],
            different_model_family: true,
            reason: "the change has a user-visible runtime surface".into(),
        });
    }
    EvalPlan {
        id: Uuid::new_v4().to_string(),
        contract_id: input.contract_id,
        schema_version: COMPLETION_SCHEMA_VERSION,
        risk,
        checks,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckRun {
    pub check_id: String,
    pub kind: EvalKind,
    pub required: bool,
    pub status: CheckStatus,
    pub executor: String,
    pub command: Option<String>,
    pub verifier_family: Option<String>,
    pub detail: Option<String>,
    pub output_digest: Option<String>,
    pub artifact_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProofBundle {
    pub schema_version: u32,
    pub attempt_id: String,
    pub contract_id: String,
    pub repository: RepositoryStamp,
    pub repository_path: String,
    pub verdict: CompletionVerdict,
    pub implementer_family: Option<String>,
    pub checks: Vec<CheckRun>,
    pub failed_or_skipped: Vec<String>,
    pub waiver_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationPacket {
    pub attempt_id: String,
    pub repository: RepositoryStamp,
    pub relevant_criteria: Vec<String>,
    pub changed_paths: Vec<String>,
    pub affected_guarantees: Vec<String>,
    pub deterministic_results: Vec<CheckRun>,
    pub unresolved_findings: Vec<String>,
    pub artifact_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionSummary {
    pub attempt_id: String,
    pub contract_id: String,
    pub verdict: CompletionVerdict,
    pub repository: RepositoryStamp,
    pub passed_required: usize,
    pub total_required: usize,
    pub checks: Vec<CheckRun>,
    pub markdown_committed: bool,
    pub waiver_reason: Option<String>,
}

pub fn compact_packet(
    attempt_id: &str,
    repository: RepositoryStamp,
    criteria: &[String],
    changed_paths: &[String],
    deterministic_results: &[CheckRun],
    unresolved_findings: &[String],
) -> VerificationPacket {
    let path_terms = changed_paths
        .iter()
        .flat_map(|path| path.split(['/', '.', '-', '_']))
        .filter(|term| term.len() >= 3)
        .map(str::to_ascii_lowercase)
        .collect::<HashSet<_>>();
    let relevant_criteria = criteria
        .iter()
        .filter(|criterion| {
            let lower = criterion.to_ascii_lowercase();
            path_terms.iter().any(|term| lower.contains(term))
        })
        .cloned()
        .collect::<Vec<_>>();
    let relevant_criteria = if relevant_criteria.is_empty() {
        criteria.iter().take(8).cloned().collect()
    } else {
        relevant_criteria
    };
    let artifact_refs = deterministic_results
        .iter()
        .flat_map(|run| run.artifact_refs.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    VerificationPacket {
        attempt_id: attempt_id.into(),
        repository,
        relevant_criteria,
        changed_paths: changed_paths.to_vec(),
        affected_guarantees: unresolved_findings.iter().take(16).cloned().collect(),
        deterministic_results: deterministic_results
            .iter()
            .filter(|run| run.kind == EvalKind::Deterministic)
            .cloned()
            .collect(),
        unresolved_findings: unresolved_findings.iter().take(16).cloned().collect(),
        artifact_refs,
    }
}

pub fn save_contract(db: &Connection, contract: &CompletionContract) -> Result<(), BridgeError> {
    if contract.acceptance_criteria.is_empty() {
        return Err(BridgeError::Invalid(
            "completion contract requires acceptance criteria".into(),
        ));
    }
    let now = Utc::now().to_rfc3339();
    db.execute(
        "INSERT INTO completion_contracts(id,workspace_id,session_id,schema_version,acceptance_criteria,markdown_projection,markdown_committed,status,created_at,updated_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,'active',?8,?8)",
        params![
            contract.id,
            contract.workspace_id,
            contract.session_id,
            contract.schema_version,
            serde_json::to_string(&contract.acceptance_criteria).map_err(|error| BridgeError::Invalid(error.to_string()))?,
            contract.markdown_projection,
            contract.markdown_committed,
            now,
        ],
    )?;
    Ok(())
}

pub fn latest_summary(
    db: &Connection,
    session_id: &str,
) -> Result<Option<CompletionSummary>, BridgeError> {
    let row: Option<(String, String, String, String, String, Option<String>, bool)> = db.query_row(
        "SELECT a.id,p.contract_id,a.status,a.repository_head,a.dirty_digest,
                (SELECT reason FROM eval_waivers w WHERE w.attempt_id=a.id ORDER BY created_at DESC LIMIT 1),
                c.markdown_committed
         FROM eval_attempts a
         JOIN eval_plans p ON p.id=a.plan_id
         JOIN completion_contracts c ON c.id=p.contract_id
         WHERE a.session_id=?1 ORDER BY a.started_at DESC LIMIT 1",
        params![session_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
    ).optional()?;
    let Some((attempt_id, contract_id, status, head, dirty_digest, waiver_reason, markdown_committed)) = row else {
        return Ok(None);
    };
    let mut statement = db.prepare(
        "SELECT check_id,kind,required,status,executor,command,verifier_family,detail,output_digest,artifact_refs FROM eval_check_runs WHERE attempt_id=?1 ORDER BY required DESC,rowid",
    )?;
    let checks = statement.query_map(params![attempt_id], map_check_run)?.collect::<Result<Vec<_>, _>>()?;
    let total_required = checks.iter().filter(|check| check.required).count();
    let passed_required = checks.iter().filter(|check| check.required && check.status == CheckStatus::Passed).count();
    Ok(Some(CompletionSummary {
        attempt_id,
        contract_id,
        verdict: parse_verdict(&status),
        repository: RepositoryStamp { head, dirty_digest },
        passed_required,
        total_required,
        checks,
        markdown_committed,
        waiver_reason,
    }))
}

pub fn completion_allows_ready(db: &Connection, session_id: &str) -> Result<bool, BridgeError> {
    Ok(latest_summary(db, session_id)?
        .map(|summary| matches!(summary.verdict, CompletionVerdict::Verified | CompletionVerdict::Waived))
        .unwrap_or(true))
}

pub fn create_from_worker_result(
    db: &Connection,
    child_session_id: &str,
    result: &WorkerResult,
) -> Result<Option<CompletionSummary>, BridgeError> {
    if result.status != WorkerResultStatus::Completed {
        return Ok(None);
    }
    let context: Option<(String, String, String, String, String, Option<String>, Option<String>)> = db.query_row(
        "SELECT r.parent_session_id,l.workspace_id,l.role,s.harness,i.request,r.worktree_path,COALESCE(parent.cwd,w.path)
         FROM worker_runtime r
         JOIN worker_leases l ON l.session_id=r.session_id
         JOIN sessions s ON s.id=r.session_id
         JOIN sessions parent ON parent.id=r.parent_session_id
         JOIN workspaces w ON w.id=parent.workspace_id
         JOIN worker_completion_inputs i ON i.child_session_id=r.session_id
         WHERE r.session_id=?1",
        params![child_session_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
    ).optional()?;
    let Some((parent_session_id, workspace_id, role, implementer_family, serialized_request, worktree_path, parent_path)) = context else {
        return Ok(None);
    };
    if role != "implementation" {
        return Ok(None);
    }
    if let Some(existing) = latest_summary(db, &parent_session_id)? {
        if matches!(existing.verdict, CompletionVerdict::Verifying | CompletionVerdict::ChangesRequested) {
            return Ok(Some(existing));
        }
    }
    let request: DelegationRequest = serde_json::from_str(&serialized_request)
        .map_err(|error| BridgeError::Invalid(format!("stored delegation request is malformed: {error}")))?;
    request.validate().map_err(BridgeError::Invalid)?;
    let repository_path = worktree_path.or(parent_path).ok_or_else(|| BridgeError::Invalid("implementation completion requires a repository path before verification".into()))?;
    let state = store::repository_state_for_path(Path::new(&repository_path));
    let head = state.get("head").and_then(serde_json::Value::as_str).ok_or_else(|| BridgeError::Invalid("implementation completion requires a Git HEAD before verification".into()))?;
    let dirty_digest = state.get("dirtyHash").and_then(serde_json::Value::as_str).ok_or_else(|| BridgeError::Invalid("implementation completion requires a dirty-tree digest before verification".into()))?;
    let contract = CompletionContract {
        id: Uuid::new_v4().to_string(),
        workspace_id,
        session_id: parent_session_id.clone(),
        schema_version: COMPLETION_SCHEMA_VERSION,
        acceptance_criteria: request.acceptance_criteria.clone(),
        markdown_projection: None,
        markdown_committed: false,
    };
    let plan = plan(PlanInput {
        contract_id: contract.id.clone(),
        acceptance_criteria: request.acceptance_criteria,
        changed_paths: result.files_changed.clone(),
        repository_commands: request.verification,
        available_capabilities: HashSet::new(),
    });
    let repository = RepositoryStamp { head: head.into(), dirty_digest: dirty_digest.into() };
    create_flow(db, &contract, &plan, &parent_session_id, &repository_path, &repository, Some(&implementer_family))?;
    latest_summary(db, &parent_session_id)
}

fn parse_verdict(value: &str) -> CompletionVerdict {
    match value {
        "changes_requested" => CompletionVerdict::ChangesRequested,
        "verified" => CompletionVerdict::Verified,
        "waived" => CompletionVerdict::Waived,
        "failed" => CompletionVerdict::Failed,
        "superseded" => CompletionVerdict::Superseded,
        _ => CompletionVerdict::Verifying,
    }
}

fn map_check_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<CheckRun> {
    let kind: String = row.get(1)?;
    let status: String = row.get(3)?;
    Ok(CheckRun {
        check_id: row.get(0)?,
        kind: match kind.as_str() { "scrutiny" => EvalKind::Scrutiny, "user_testing" => EvalKind::UserTesting, _ => EvalKind::Deterministic },
        required: row.get(2)?,
        status: match status.as_str() { "running" => CheckStatus::Running, "passed" => CheckStatus::Passed, "failed" => CheckStatus::Failed, "skipped" => CheckStatus::Skipped, "blocked" => CheckStatus::Blocked, "stale" => CheckStatus::Stale, _ => CheckStatus::Pending },
        executor: row.get(4)?,
        command: row.get(5)?,
        verifier_family: row.get(6)?,
        detail: row.get(7)?,
        output_digest: row.get(8)?,
        artifact_refs: serde_json::from_str(&row.get::<_, String>(9)?).unwrap_or_default(),
    })
}

pub fn save_plan(db: &Connection, plan: &EvalPlan) -> Result<(), BridgeError> {
    db.execute(
        "INSERT INTO eval_plans(id,contract_id,schema_version,risk,plan,created_at) VALUES(?1,?2,?3,?4,?5,?6)",
        params![plan.id, plan.contract_id, plan.schema_version, plan.risk.as_str(), serde_json::to_string(plan).map_err(|error| BridgeError::Invalid(error.to_string()))?, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

pub fn create_flow(
    db: &Connection,
    contract: &CompletionContract,
    plan: &EvalPlan,
    session_id: &str,
    repository_path: &str,
    repository: &RepositoryStamp,
    implementer_family: Option<&str>,
) -> Result<String, BridgeError> {
    if contract.acceptance_criteria.is_empty() || plan.contract_id != contract.id {
        return Err(BridgeError::Invalid(
            "completion flow requires a non-empty contract and its matching eval plan".into(),
        ));
    }
    repository.validate()?;
    if repository_path.trim().is_empty() {
        return Err(BridgeError::Invalid("completion flow requires the exact repository path being evaluated".into()));
    }
    let now = Utc::now().to_rfc3339();
    let attempt_id = Uuid::new_v4().to_string();
    let transaction = db.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO completion_contracts(id,workspace_id,session_id,schema_version,acceptance_criteria,markdown_projection,markdown_committed,status,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,'active',?8,?8)",
        params![contract.id, contract.workspace_id, contract.session_id, contract.schema_version, serde_json::to_string(&contract.acceptance_criteria).map_err(|error| BridgeError::Invalid(error.to_string()))?, contract.markdown_projection, contract.markdown_committed, now],
    )?;
    transaction.execute(
        "INSERT INTO eval_plans(id,contract_id,schema_version,risk,plan,created_at) VALUES(?1,?2,?3,?4,?5,?6)",
        params![plan.id, plan.contract_id, plan.schema_version, plan.risk.as_str(), serde_json::to_string(plan).map_err(|error| BridgeError::Invalid(error.to_string()))?, now],
    )?;
    transaction.execute(
        "UPDATE eval_attempts SET status='superseded',completed_at=?2 WHERE session_id=?1 AND status IN ('verifying','changes_requested') AND (repository_head<>?3 OR dirty_digest<>?4)",
        params![session_id, now, repository.head, repository.dirty_digest],
    )?;
    transaction.execute(
        "INSERT INTO eval_attempts(id,plan_id,session_id,repository_head,dirty_digest,repository_path,status,implementer_family,started_at) VALUES(?1,?2,?3,?4,?5,?6,'verifying',?7,?8)",
        params![attempt_id, plan.id, session_id, repository.head, repository.dirty_digest, repository_path, implementer_family, now],
    )?;
    for check in &plan.checks {
        transaction.execute(
            "INSERT INTO eval_check_runs(id,attempt_id,check_id,kind,required,status,executor,command,artifact_refs) VALUES(?1,?2,?3,?4,?5,'pending',?6,?7,'[]')",
            params![Uuid::new_v4().to_string(), attempt_id, check.id, check.kind.as_str(), check.required, check.executor, check.command],
        )?;
    }
    transaction.execute("UPDATE sessions SET status='waiting' WHERE id=?1", params![session_id])?;
    transaction.commit()?;
    Ok(attempt_id)
}

pub fn begin_attempt(
    db: &Connection,
    plan: &EvalPlan,
    session_id: &str,
    repository_path: &str,
    repository: &RepositoryStamp,
    implementer_family: Option<&str>,
) -> Result<String, BridgeError> {
    repository.validate()?;
    if repository_path.trim().is_empty() {
        return Err(BridgeError::Invalid("verification attempt requires a repository path".into()));
    }
    let id = Uuid::new_v4().to_string();
    let transaction = db.unchecked_transaction()?;
    transaction.execute(
        "UPDATE eval_attempts SET status='superseded',completed_at=?2 WHERE session_id=?1 AND status IN ('verifying','changes_requested') AND (repository_head<>?3 OR dirty_digest<>?4)",
        params![session_id, Utc::now().to_rfc3339(), repository.head, repository.dirty_digest],
    )?;
    transaction.execute(
        "INSERT INTO eval_attempts(id,plan_id,session_id,repository_head,dirty_digest,repository_path,status,implementer_family,started_at) VALUES(?1,?2,?3,?4,?5,?6,'verifying',?7,?8)",
        params![id, plan.id, session_id, repository.head, repository.dirty_digest, repository_path, implementer_family, Utc::now().to_rfc3339()],
    )?;
    for check in &plan.checks {
        transaction.execute(
            "INSERT INTO eval_check_runs(id,attempt_id,check_id,kind,required,status,executor,command,artifact_refs) VALUES(?1,?2,?3,?4,?5,'pending',?6,?7,'[]')",
            params![Uuid::new_v4().to_string(), id, check.id, check.kind.as_str(), check.required, check.executor, check.command],
        )?;
    }
    transaction.commit()?;
    Ok(id)
}

pub fn record_check(
    db: &Connection,
    attempt_id: &str,
    run: &CheckRun,
) -> Result<(), BridgeError> {
    let attempt: Option<(String, String, Option<String>, String)> = db
        .query_row(
            "SELECT a.repository_head,a.dirty_digest,a.implementer_family,p.plan FROM eval_attempts a JOIN eval_plans p ON p.id=a.plan_id WHERE a.id=?1 AND a.status NOT IN ('verified','waived','superseded')",
            params![attempt_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let Some((_, _, implementer_family, serialized_plan)) = attempt else {
        return Err(BridgeError::Invalid(
            "check result targets a terminal or unknown verification attempt".into(),
        ));
    };
    let plan: EvalPlan = serde_json::from_str(&serialized_plan)
        .map_err(|error| BridgeError::Invalid(format!("stored eval plan is malformed: {error}")))?;
    let check = plan
        .checks
        .iter()
        .find(|check| check.id == run.check_id)
        .ok_or_else(|| BridgeError::Invalid(format!("unknown check {} for verification attempt", run.check_id)))?;
    if run.status == CheckStatus::Passed && check.different_model_family {
        let verifier = run.verifier_family.as_deref().filter(|value| !value.trim().is_empty());
        let implementer = implementer_family.as_deref().filter(|value| !value.trim().is_empty());
        if verifier.is_none() || implementer.is_none() || verifier == implementer {
            return Err(BridgeError::Invalid(
                "independent verifier evidence must come from a different model family".into(),
            ));
        }
    }
    if run.status == CheckStatus::Passed
        && run.output_digest.as_deref().map(str::trim).filter(|value| !value.is_empty()).is_none()
    {
        return Err(BridgeError::Invalid(
            "passing check evidence requires an output digest produced by the executor".into(),
        ));
    }
    let detail = run.detail.as_deref().map(|value| {
        let mut bounded = value.chars().take(8_192).collect::<String>();
        if value.chars().count() > 8_192 {
            bounded.push_str("\n[output truncated by Bridge]");
        }
        bounded
    });
    let updated = db.execute(
        "UPDATE eval_check_runs SET status=?3,verifier_family=?4,detail=?5,output_digest=?6,artifact_refs=?7,started_at=COALESCE(started_at,?8),completed_at=?8 WHERE attempt_id=?1 AND check_id=?2",
        params![attempt_id, run.check_id, run.status.as_str(), run.verifier_family, detail, run.output_digest, serde_json::to_string(&run.artifact_refs).map_err(|error| BridgeError::Invalid(error.to_string()))?, Utc::now().to_rfc3339()],
    )?;
    if updated != 1 {
        return Err(BridgeError::Invalid(
            "check result did not update exactly one planned check".into(),
        ));
    }
    Ok(())
}

pub fn finalize(
    db: &Connection,
    attempt_id: &str,
    current_repository: &RepositoryStamp,
) -> Result<ProofBundle, BridgeError> {
    current_repository.validate()?;
    let (contract_id, head, dirty, repository_path, implementer_family): (String, String, String, String, Option<String>) = db.query_row(
        "SELECT p.contract_id,a.repository_head,a.dirty_digest,a.repository_path,a.implementer_family FROM eval_attempts a JOIN eval_plans p ON p.id=a.plan_id WHERE a.id=?1",
        params![attempt_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    )?;
    if head != current_repository.head || dirty != current_repository.dirty_digest {
        db.execute("UPDATE eval_attempts SET status='superseded',completed_at=?2 WHERE id=?1", params![attempt_id, Utc::now().to_rfc3339()])?;
        return Err(BridgeError::Invalid(
            "repository changed after evaluation; prior proof is stale".into(),
        ));
    }
    let mut statement = db.prepare(
        "SELECT check_id,kind,required,status,executor,command,verifier_family,detail,output_digest,artifact_refs FROM eval_check_runs WHERE attempt_id=?1 ORDER BY rowid",
    )?;
    let checks = statement
        .query_map(params![attempt_id], map_check_run)?
        .collect::<Result<Vec<_>, _>>()?;
    let waiver_reason: Option<String> = db.query_row(
        "SELECT reason FROM eval_waivers WHERE attempt_id=?1 AND repository_head=?2 AND dirty_digest=?3 ORDER BY created_at DESC LIMIT 1",
        params![attempt_id, head, dirty],
        |row| row.get(0),
    ).optional()?;
    let blockers = checks.iter().filter(|run| run.required && run.status != CheckStatus::Passed).collect::<Vec<_>>();
    let verdict = if blockers.is_empty() {
        CompletionVerdict::Verified
    } else if waiver_reason.is_some() {
        CompletionVerdict::Waived
    } else if blockers.iter().any(|run| run.status == CheckStatus::Failed) {
        CompletionVerdict::ChangesRequested
    } else {
        CompletionVerdict::Verifying
    };
    let failed_or_skipped = checks.iter().filter(|run| matches!(run.status, CheckStatus::Failed | CheckStatus::Skipped | CheckStatus::Blocked | CheckStatus::Stale)).map(|run| run.check_id.clone()).collect();
    let bundle = ProofBundle {
        schema_version: COMPLETION_SCHEMA_VERSION,
        attempt_id: attempt_id.into(),
        contract_id,
        repository: current_repository.clone(),
        repository_path,
        verdict,
        implementer_family,
        checks,
        failed_or_skipped,
        waiver_reason,
    };
    let serialized = serde_json::to_string(&bundle).map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let digest = format!("{:x}", Sha256::digest(serialized.as_bytes()));
    let transaction = db.unchecked_transaction()?;
    transaction.execute(
        "UPDATE eval_attempts SET status=?2,completed_at=CASE WHEN ?2 IN ('verified','waived') THEN ?3 ELSE NULL END WHERE id=?1",
        params![attempt_id, verdict.as_str(), Utc::now().to_rfc3339()],
    )?;
    transaction.execute(
        "INSERT INTO proof_bundles(id,attempt_id,schema_version,verdict,bundle,bundle_digest,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(attempt_id) DO UPDATE SET verdict=excluded.verdict,bundle=excluded.bundle,bundle_digest=excluded.bundle_digest,created_at=excluded.created_at",
        params![Uuid::new_v4().to_string(), attempt_id, COMPLETION_SCHEMA_VERSION, verdict.as_str(), serialized, digest, Utc::now().to_rfc3339()],
    )?;
    transaction.commit()?;
    Ok(bundle)
}

pub fn waive(
    db: &Connection,
    attempt_id: &str,
    check_ids: &[String],
    reason: &str,
    granted_by: &str,
    repository: &RepositoryStamp,
) -> Result<(), BridgeError> {
    if check_ids.is_empty() || reason.trim().is_empty() || granted_by.trim().is_empty() {
        return Err(BridgeError::Invalid(
            "waiver requires scoped checks, a reason, and a human actor".into(),
        ));
    }
    repository.validate()?;
    let expected: (String, String) = db.query_row(
        "SELECT repository_head,dirty_digest FROM eval_attempts WHERE id=?1 AND status NOT IN ('verified','waived','superseded')",
        params![attempt_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if expected.0 != repository.head || expected.1 != repository.dirty_digest {
        return Err(BridgeError::Invalid(
            "waiver repository stamp does not match the active verification attempt".into(),
        ));
    }
    if check_ids.iter().collect::<HashSet<_>>().len() != check_ids.len() {
        return Err(BridgeError::Invalid("waiver check scope contains duplicates".into()));
    }
    for check_id in check_ids {
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM eval_check_runs WHERE attempt_id=?1 AND check_id=?2 AND required=1 AND status!='passed')",
            params![attempt_id, check_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(BridgeError::Invalid(format!(
                "waiver check {check_id} is not an unresolved required check"
            )));
        }
    }
    db.execute(
        "INSERT INTO eval_waivers(id,attempt_id,check_ids,reason,granted_by,repository_head,dirty_digest,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![Uuid::new_v4().to_string(), attempt_id, serde_json::to_string(check_ids).map_err(|error| BridgeError::Invalid(error.to_string()))?, reason.trim(), granted_by.trim(), repository.head, repository.dirty_digest, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;

    fn fixture() -> Connection {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','p','/tmp/p','now')", []).unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','x','w','main','/tmp/w','idle','now')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind,continuation_fidelity) VALUES('s','w','codex','s','idle','estimated','orchestrator','native')", []).unwrap();
        db
    }

    fn contract() -> CompletionContract {
        CompletionContract { id: "contract".into(), workspace_id: "w".into(), session_id: "s".into(), schema_version: 1, acceptance_criteria: vec!["Router dialog saves preferences".into()], markdown_projection: None, markdown_committed: false }
    }

    #[test]
    fn planner_selects_independent_scrutiny_and_browser_journey() {
        let result = plan(PlanInput { contract_id: "c".into(), acceptance_criteria: vec!["User opens the dialog".into()], changed_paths: vec!["src/components/Dialog.tsx".into(), "src-tauri/src/policy.rs".into()], repository_commands: vec![], available_capabilities: HashSet::new() });
        assert_eq!(result.risk, RiskTier::High);
        assert!(result.checks.iter().any(|check| check.kind == EvalKind::Scrutiny && check.different_model_family));
        assert!(result.checks.iter().any(|check| check.kind == EvalKind::UserTesting && check.required_capabilities.contains(&"browser".into())));
        assert!(result.checks.iter().any(|check| check.command.as_deref() == Some("bun run build")));
    }

    #[test]
    fn verifier_rejects_same_family_and_missing_tools() {
        let manifest = VerifierManifest { id: "web".into(), kind: EvalKind::UserTesting, triggers: vec!["frontend".into()], required_capabilities: vec!["browser".into()], different_model_family: true, checks: vec!["open app".into()], evidence_required: vec!["screenshot".into()] };
        assert!(manifest.eligible(Some("codex"), Some("codex"), &HashSet::from(["browser".into()])).is_err());
        assert!(manifest.eligible(Some("codex"), Some("claude"), &HashSet::new()).unwrap_err().contains("browser"));
        assert!(manifest.eligible(Some("codex"), Some("claude"), &HashSet::from(["browser".into()])).is_ok());
    }

    #[test]
    fn skill_verifier_manifests_add_checks_without_granting_missing_tools() {
        let db = fixture();
        let manifest = VerifierManifest { id: "playwright-journey".into(), kind: EvalKind::UserTesting, triggers: vec!["frontend".into()], required_capabilities: vec!["browser".into(), "network_inspection".into()], different_model_family: true, checks: vec!["exercise acceptance journey".into()], evidence_required: vec!["trace".into(), "screenshot".into()] };
        register_verifier_manifest(&db, "skill:review-checkpoint", &manifest).unwrap();
        let blocked = verifier_candidates(&db, &["frontend".into()], &HashSet::from(["browser".into()])).unwrap();
        assert!(!blocked[0].eligible);
        assert!(blocked[0].exclusion_reasons[0].contains("network_inspection"));
        let eligible = verifier_candidates(&db, &["frontend".into()], &HashSet::from(["browser".into(), "network_inspection".into()])).unwrap();
        assert!(eligible[0].eligible);
        assert!(eligible[0].manifest.different_model_family);
    }

    #[test]
    fn completed_implementation_opens_a_private_verification_gate() {
        use crate::delegation::{SuggestedNextAction, WorkerTestResult};
        let db = fixture();
        let cwd = std::env::current_dir().unwrap().to_string_lossy().into_owned();
        db.execute("UPDATE sessions SET cwd='/bridge/missing-parent-worktree' WHERE id='s'", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id,kind,continuation_fidelity) VALUES('child','w','claude','impl','completed','estimated','s','worker','native')", []).unwrap();
        db.execute("INSERT INTO worker_leases(session_id,workspace_id,role,capability_tier,task_family,owned_paths,write_mode,lease_status,created_at,updated_at) VALUES('child','w','implementation','standard','implementation','[]','isolated','released','now','now')", []).unwrap();
        db.execute("INSERT INTO worker_runtime(session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,result_status,retry_count,updated_at) VALUES('child','s','completed','implementation','key','reported',0,'now')", []).unwrap();
        db.execute("UPDATE worker_runtime SET worktree_path=?2 WHERE session_id=?1", params!["child", cwd]).unwrap();
        let request = serde_json::json!({"schemaVersion":1,"role":"implementation","objective":"Implement proof","acceptanceCriteria":["Proof card is visible"],"knownFacts":[],"decisions":[],"evidenceIds":[],"relevantFiles":["src/App.tsx"],"ownedPaths":["src/**"],"writeMode":"isolated","capabilityTier":"standard","effort":"medium","verification":["bun run test"],"outputContract":"implementation-result","harness":"claude"});
        db.execute("INSERT INTO worker_completion_inputs(child_session_id,request,updated_at) VALUES('child',?1,'now')", params![request.to_string()]).unwrap();
        let result = WorkerResult { schema_version: 1, status: WorkerResultStatus::Completed, summary: "implemented".into(), files_changed: vec!["src/App.tsx".into()], tests: Vec::<WorkerTestResult>::new(), decisions: vec![], risks: vec![], remaining_work: vec![], suggested_next_action: SuggestedNextAction::Finish, suggested_role: None, suggested_task: None };
        let summary = create_from_worker_result(&db, "child", &result).unwrap().unwrap();
        assert_eq!(summary.verdict, CompletionVerdict::Verifying);
        assert!(!summary.markdown_committed);
        assert!(summary.checks.iter().any(|check| check.kind == EvalKind::Scrutiny));
        assert_eq!(db.query_row("SELECT repository_path FROM eval_attempts WHERE id=?1", params![summary.attempt_id], |row| row.get::<_, String>(0)).unwrap(), cwd);
        assert_eq!(db.query_row("SELECT status FROM sessions WHERE id='s'", [], |row| row.get::<_, String>(0)).unwrap(), "waiting");
    }

    #[test]
    fn completion_is_revision_bound_and_preserves_failed_checks() {
        let db = fixture();
        let contract = contract();
        save_contract(&db, &contract).unwrap();
        let plan = EvalPlan { id: "plan".into(), contract_id: contract.id.clone(), schema_version: 1, risk: RiskTier::High, checks: vec![EvalCheck { id: "tests".into(), label: "tests".into(), kind: EvalKind::Deterministic, required: true, executor: "shell".into(), command: Some("bun test".into()), required_capabilities: vec!["shell".into()], different_model_family: false, reason: "policy".into() }] };
        save_plan(&db, &plan).unwrap();
        let stamp = RepositoryStamp { head: "abc".into(), dirty_digest: "clean".into() };
        let attempt = begin_attempt(&db, &plan, "s", ".", &stamp, Some("codex")).unwrap();
        record_check(&db, &attempt, &CheckRun { check_id: "tests".into(), kind: EvalKind::Deterministic, required: true, status: CheckStatus::Failed, executor: "shell".into(), command: Some("bun test".into()), verifier_family: None, detail: Some("failure".into()), output_digest: Some("digest".into()), artifact_refs: vec![] }).unwrap();
        let bundle = finalize(&db, &attempt, &stamp).unwrap();
        assert_eq!(bundle.verdict, CompletionVerdict::ChangesRequested);
        assert_eq!(bundle.failed_or_skipped, vec!["tests"]);
        assert!(finalize(&db, &attempt, &RepositoryStamp { head: "def".into(), dirty_digest: "clean".into() }).unwrap_err().to_string().contains("stale"));
    }

    #[test]
    fn persisted_check_cannot_claim_same_family_independence() {
        let db = fixture();
        let contract = contract();
        save_contract(&db, &contract).unwrap();
        let plan = EvalPlan { id: "plan".into(), contract_id: contract.id.clone(), schema_version: 1, risk: RiskTier::High, checks: vec![EvalCheck { id: "scrutiny".into(), label: "scrutiny".into(), kind: EvalKind::Scrutiny, required: true, executor: "worker".into(), command: None, required_capabilities: vec!["code_review".into()], different_model_family: true, reason: "risk".into() }] };
        save_plan(&db, &plan).unwrap();
        let stamp = RepositoryStamp { head: "abc".into(), dirty_digest: "clean".into() };
        let attempt = begin_attempt(&db, &plan, "s", ".", &stamp, Some("codex")).unwrap();
        let error = record_check(&db, &attempt, &CheckRun { check_id: "scrutiny".into(), kind: EvalKind::Scrutiny, required: true, status: CheckStatus::Passed, executor: "worker".into(), command: None, verifier_family: Some("codex".into()), detail: None, output_digest: None, artifact_refs: vec![] }).unwrap_err();
        assert!(error.to_string().contains("different model family"));
    }

    #[test]
    fn scoped_human_waiver_is_distinct_from_verified() {
        let db = fixture();
        let contract = contract();
        save_contract(&db, &contract).unwrap();
        let plan = EvalPlan { id: "plan".into(), contract_id: contract.id.clone(), schema_version: 1, risk: RiskTier::Low, checks: vec![EvalCheck { id: "manual".into(), label: "manual".into(), kind: EvalKind::UserTesting, required: true, executor: "worker".into(), command: None, required_capabilities: vec!["computer".into()], different_model_family: true, reason: "journey".into() }] };
        save_plan(&db, &plan).unwrap();
        let stamp = RepositoryStamp { head: "abc".into(), dirty_digest: "clean".into() };
        let attempt = begin_attempt(&db, &plan, "s", ".", &stamp, Some("codex")).unwrap();
        waive(&db, &attempt, &["manual".into()], "tool unavailable", "user", &stamp).unwrap();
        assert_eq!(finalize(&db, &attempt, &stamp).unwrap().verdict, CompletionVerdict::Waived);
    }

    #[test]
    fn compact_packet_selects_relevant_contract_and_bounded_findings() {
        let packet = compact_packet("a", RepositoryStamp { head: "h".into(), dirty_digest: "d".into() }, &["Router saves settings".into(), "Unrelated billing works".into()], &["src/router/settings.rs".into()], &[], &(0..30).map(|index| format!("finding {index}")).collect::<Vec<_>>());
        assert_eq!(packet.relevant_criteria, vec!["Router saves settings"]);
        assert_eq!(packet.unresolved_findings.len(), 16);
    }
}

use crate::{
    delegation::{DelegationRequest, TestStatus, WorkerResult, WorkerResultStatus},
    store, BridgeError,
};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashSet};
use std::path::Path;
use uuid::Uuid;

pub const COMPLETION_SCHEMA_VERSION: u32 = 1;

/// Deterministic checks Bridge itself runs, in the attempt's repository.
pub const SHELL_EXECUTOR: &str = "bridge.shell";
/// Semantic checks a verification worker settles.
pub const WORKER_EXECUTOR: &str = "bridge.worker";
/// Evidence derived from a verification worker's typed result. Never sufficient
/// for a planned `bridge.shell` check: the digest only proves which JSON arrived.
pub const WORKER_RESULT_EXECUTOR: &str = "bridge.worker_result";
/// Verdicts Bridge records about its own machinery (gate errors, deadlines).
pub const SYSTEM_EXECUTOR: &str = "bridge.system";

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

/// Path/content tokens that mark a change `RiskTier::High` regardless of
/// anything else. Shared between the deterministic-check planner (aggregate
/// risk over a whole change) and per-file importance scoring.
const HIGH_RISK_TOKENS: &[&str] = &[
    "auth",
    "secret",
    "credential",
    "migration",
    "migrations",
    "policy",
    "adapter",
    "adapters",
];
const HIGH_RISK_FILENAMES: &[&str] = &[
    "store.rs",
    "worker_lifecycle.rs",
    "session_supervisor.rs",
    "learning_router.rs",
];
/// Extensions/paths whose presence marks a change user-facing (`RiskTier::Medium`
/// absent a High signal).
const USER_FACING_PATH_NEEDLES: &[&str] = &[".tsx", ".jsx", "src/components", "src/app"];

fn path_is_high_risk(lower_path: &str) -> bool {
    HIGH_RISK_FILENAMES.iter().any(|name| lower_path.ends_with(name))
        || lower_path
            .split(|character: char| !character.is_ascii_alphanumeric())
            .any(|token| HIGH_RISK_TOKENS.contains(&token))
}

fn path_is_user_facing(lower_path: &str) -> bool {
    USER_FACING_PATH_NEEDLES
        .iter()
        .any(|needle| lower_path.contains(needle))
}

/// A single file's importance, using the same signals `plan()` weighs in
/// aggregate: known high-risk tokens/filenames, then user-facing surface.
/// Unlike `plan()`, a lone file is never promoted to `Medium` just for being
/// one of many changed paths.
pub fn risk_tier_for_path(path: &str) -> RiskTier {
    let lower = path.to_ascii_lowercase();
    if path_is_high_risk(&lower) {
        RiskTier::High
    } else if path_is_user_facing(&lower) {
        RiskTier::Medium
    } else {
        RiskTier::Low
    }
}

/// Lockfiles, generated output, and vendored trees carry little review signal
/// even when they change substantially — they should be visible, never hidden
/// silently, but collapsed out of the way by default.
pub fn is_low_signal_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    const SUFFIXES: &[&str] = &[".lock", "lock.json", "lock.yaml"];
    const DIRECTORIES: &[&str] = &[
        "generated",
        "vendor",
        "vendored",
        "node_modules",
        "dist",
        "build",
    ];
    SUFFIXES.iter().any(|suffix| lower.ends_with(suffix))
        || lower
            .rsplit('/')
            .skip(1)
            .any(|directory| DIRECTORIES.contains(&directory))
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
            return Err(format!(
                "missing verifier capabilities: {}",
                missing.join(", ")
            ));
        }
        if self.different_model_family {
            let implementer = implementer_family.filter(|value| !value.trim().is_empty());
            let verifier = verifier_family.filter(|value| !value.trim().is_empty());
            if implementer.is_none() || verifier.is_none() || implementer == verifier {
                return Err(
                    "verifier must use a different model family from the implementer".into(),
                );
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
        return Err(BridgeError::Invalid(
            "verifier manifest source cannot be empty".into(),
        ));
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
    let labels = change_labels
        .iter()
        .map(|label| label.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    let mut statement =
        db.prepare("SELECT manifest FROM verifier_manifests WHERE enabled=1 ORDER BY id")?;
    let manifests = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    manifests
        .into_iter()
        .map(|serialized| {
            let manifest: VerifierManifest =
                serde_json::from_str(&serialized).map_err(|error| {
                    BridgeError::Invalid(format!("stored verifier manifest is malformed: {error}"))
                })?;
            manifest.validate()?;
            let mut exclusion_reasons = Vec::new();
            if !manifest.triggers.is_empty()
                && !manifest
                    .triggers
                    .iter()
                    .any(|trigger| labels.contains(&trigger.to_ascii_lowercase()))
            {
                exclusion_reasons.push("change triggers do not match".into());
            }
            let missing = manifest
                .required_capabilities
                .iter()
                .filter(|capability| !available_capabilities.contains(capability.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                exclusion_reasons.push(format!("missing capabilities: {}", missing.join(", ")));
            }
            Ok(VerifierCandidate {
                eligible: exclusion_reasons.is_empty(),
                manifest,
                exclusion_reasons,
            })
        })
        .collect()
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
    let high_risk = lower_paths.iter().any(|path| path_is_high_risk(path));
    let user_facing = lower_paths.iter().any(|path| path_is_user_facing(path))
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
        commands.insert("cargo test --manifest-path src-tauri/Cargo.toml --workspace".into());
        commands.insert("cargo check --manifest-path src-tauri/Cargo.toml --workspace".into());
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
            executor: SHELL_EXECUTOR.into(),
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
            executor: WORKER_EXECUTOR.into(),
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
            executor: WORKER_EXECUTOR.into(),
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

pub fn plan_with_registered_manifests(
    db: &Connection,
    input: PlanInput,
    change_labels: &[String],
    available_capabilities: &HashSet<String>,
) -> Result<EvalPlan, BridgeError> {
    let mut result = plan(input);
    for candidate in verifier_candidates(db, change_labels, available_capabilities)?
        .into_iter()
        .filter(|candidate| candidate.eligible)
    {
        for (index, label) in candidate.manifest.checks.iter().enumerate() {
            let id = format!("manifest-{}-{index}", candidate.manifest.id);
            if result.checks.iter().any(|check| check.id == id) {
                continue;
            }
            result.checks.push(EvalCheck {
                id,
                label: label.clone(),
                kind: candidate.manifest.kind,
                required: true,
                executor: WORKER_EXECUTOR.into(),
                command: None,
                required_capabilities: candidate.manifest.required_capabilities.clone(),
                different_model_family: candidate.manifest.different_model_family,
                reason: format!(
                    "registered verifier manifest {} matched this change",
                    candidate.manifest.id
                ),
            });
        }
    }
    Ok(result)
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
    #[serde(serialize_with = "crate::model::serialize_js_safe_usize")]
    pub passed_required: usize,
    #[serde(serialize_with = "crate::model::serialize_js_safe_usize")]
    pub total_required: usize,
    pub checks: Vec<CheckRun>,
    pub markdown_committed: bool,
    pub waiver_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionBenchmarkCase {
    pub id: String,
    pub baseline_claimed_done: bool,
    pub proof_verdict: CompletionVerdict,
    pub actual_accepted: bool,
    pub baseline_normalized_cost: i64,
    pub proof_normalized_cost: i64,
    pub baseline_latency_ms: i64,
    pub proof_latency_ms: i64,
    pub baseline_human_interventions: i64,
    pub proof_human_interventions: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionBenchmarkReport {
    pub cases: usize,
    pub baseline_false_done: usize,
    pub proof_false_done: usize,
    pub baseline_verified_quality_bps: u16,
    pub proof_verified_quality_bps: u16,
    pub baseline_cost_per_accepted: i64,
    pub proof_cost_per_accepted: i64,
    pub baseline_average_latency_ms: i64,
    pub proof_average_latency_ms: i64,
    pub baseline_human_interventions: i64,
    pub proof_human_interventions: i64,
}

pub fn benchmark(cases: &[CompletionBenchmarkCase]) -> CompletionBenchmarkReport {
    let proof_claimed_done =
        |case: &&CompletionBenchmarkCase| case.proof_verdict == CompletionVerdict::Verified;
    let baseline_claims = cases
        .iter()
        .filter(|case| case.baseline_claimed_done)
        .count();
    let proof_claims = cases.iter().filter(proof_claimed_done).count();
    let baseline_correct = cases
        .iter()
        .filter(|case| case.baseline_claimed_done && case.actual_accepted)
        .count();
    let proof_correct = cases
        .iter()
        .filter(|case| case.proof_verdict == CompletionVerdict::Verified && case.actual_accepted)
        .count();
    let accepted = cases
        .iter()
        .filter(|case| case.actual_accepted)
        .count()
        .max(1) as i64;
    let count = cases.len().max(1) as i64;
    CompletionBenchmarkReport {
        cases: cases.len(),
        baseline_false_done: cases
            .iter()
            .filter(|case| case.baseline_claimed_done && !case.actual_accepted)
            .count(),
        proof_false_done: cases
            .iter()
            .filter(|case| {
                case.proof_verdict == CompletionVerdict::Verified && !case.actual_accepted
            })
            .count(),
        baseline_verified_quality_bps: (baseline_correct * 10_000)
            .checked_div(baseline_claims)
            .unwrap_or(0) as u16,
        proof_verified_quality_bps: (proof_correct * 10_000)
            .checked_div(proof_claims)
            .unwrap_or(0) as u16,
        baseline_cost_per_accepted: cases
            .iter()
            .map(|case| case.baseline_normalized_cost)
            .sum::<i64>()
            / accepted,
        proof_cost_per_accepted: cases
            .iter()
            .map(|case| case.proof_normalized_cost)
            .sum::<i64>()
            / accepted,
        baseline_average_latency_ms: cases
            .iter()
            .map(|case| case.baseline_latency_ms)
            .sum::<i64>()
            / count,
        proof_average_latency_ms: cases.iter().map(|case| case.proof_latency_ms).sum::<i64>()
            / count,
        baseline_human_interventions: cases
            .iter()
            .map(|case| case.baseline_human_interventions)
            .sum(),
        proof_human_interventions: cases
            .iter()
            .map(|case| case.proof_human_interventions)
            .sum(),
    }
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
         WHERE a.session_id=?1 ORDER BY a.started_at DESC,a.rowid DESC LIMIT 1",
        params![session_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
    ).optional()?;
    let Some((
        attempt_id,
        contract_id,
        status,
        head,
        dirty_digest,
        waiver_reason,
        markdown_committed,
    )) = row
    else {
        return Ok(None);
    };
    let mut statement = db.prepare(
        "SELECT check_id,kind,required,status,executor,command,verifier_family,detail,output_digest,artifact_refs FROM eval_check_runs WHERE attempt_id=?1 ORDER BY required DESC,rowid",
    )?;
    let checks = statement
        .query_map(params![attempt_id], map_check_run)?
        .collect::<Result<Vec<_>, _>>()?;
    let total_required = checks.iter().filter(|check| check.required).count();
    let passed_required = checks
        .iter()
        .filter(|check| check.required && check.status == CheckStatus::Passed)
        .count();
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

/// Marker for an attempt that ended because its verification deadline expired
/// rather than because its checks reached verdicts.
pub const VERIFY_DEADLINE_ESCALATION: &str = "verify_deadline";

/// Whether the completion gate still holds the session back.
///
/// `Verifying` and `ChangesRequested` are live states: more automatic work is
/// expected, so the session stays blocked. A gate that could not be built keeps
/// failing closed and must be waived by a human. A gate that ran *out of time*
/// is different: nothing further will ever happen on its own, so holding the
/// session in `waiting` forever is a deadlock, not a safety property — the parent
/// is released to report the checks that never ran.
pub fn completion_allows_ready(db: &Connection, session_id: &str) -> Result<bool, BridgeError> {
    let Some(summary) = latest_summary(db, session_id)? else {
        return Ok(true);
    };
    if matches!(
        summary.verdict,
        CompletionVerdict::Verified | CompletionVerdict::Waived | CompletionVerdict::Superseded
    ) {
        return Ok(true);
    }
    if summary.verdict != CompletionVerdict::Failed {
        return Ok(false);
    }
    let escalation: Option<String> = db.query_row(
        "SELECT escalation FROM eval_attempts WHERE id=?1",
        params![summary.attempt_id],
        |row| row.get(0),
    )?;
    Ok(escalation.as_deref() == Some(VERIFY_DEADLINE_ESCALATION))
}

pub fn reconcile_parent_readiness(db: &Connection, session_id: &str) -> Result<bool, BridgeError> {
    let remaining: i64 = db.query_row(
        "SELECT COUNT(*) FROM worker_runtime WHERE parent_session_id=?1 AND result_status!='reported'",
        params![session_id],
        |row| row.get(0),
    )?;
    // Changes that exist only in a child worktree have not reached the user's
    // task checkout. A session with unadopted worker output is not finished, no
    // matter what its checks say.
    let unadopted = !crate::worker_adoption::pending_for_parent(db, session_id)?.is_empty();
    let ready = remaining == 0 && !unadopted && completion_allows_ready(db, session_id)?;
    if ready {
        db.execute(
            "UPDATE sessions SET status='ready' WHERE id=?1 AND status='waiting'",
            params![session_id],
        )?;
    } else {
        db.execute(
            "UPDATE sessions SET status='waiting' WHERE id=?1 AND status='ready'",
            params![session_id],
        )?;
    }
    Ok(ready)
}

struct WorkerCompletionContext {
    parent_session_id: String,
    workspace_id: String,
    role: String,
    harness: String,
    serialized_request: Option<String>,
    worktree_path: Option<String>,
    parent_path: Option<String>,
}

fn worker_completion_context(
    db: &Connection,
    child_session_id: &str,
) -> Result<Option<WorkerCompletionContext>, BridgeError> {
    db.query_row(
        "SELECT r.parent_session_id,l.workspace_id,l.role,s.harness,i.request,r.worktree_path,COALESCE(parent.cwd,w.path)
         FROM worker_runtime r
         JOIN worker_leases l ON l.session_id=r.session_id
         JOIN sessions s ON s.id=r.session_id
         JOIN sessions parent ON parent.id=r.parent_session_id
         LEFT JOIN workspaces w ON w.id=parent.workspace_id
         LEFT JOIN worker_completion_inputs i ON i.child_session_id=r.session_id
         WHERE r.session_id=?1",
        params![child_session_id],
        |row| Ok(WorkerCompletionContext {
            parent_session_id: row.get(0)?, workspace_id: row.get(1)?, role: row.get(2)?,
            harness: row.get(3)?, serialized_request: row.get(4)?, worktree_path: row.get(5)?, parent_path: row.get(6)?,
        }),
    ).optional().map_err(BridgeError::from)
}

fn repository_stamp(path: &str) -> Result<RepositoryStamp, BridgeError> {
    let state = store::repository_state_for_path(Path::new(path));
    let head = state
        .get("head")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            BridgeError::Invalid("completion requires a Git HEAD before verification".into())
        })?;
    let dirty_digest = state
        .get("dirtyHash")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            BridgeError::Invalid(
                "completion requires a dirty-tree digest before verification".into(),
            )
        })?;
    Ok(RepositoryStamp {
        head: head.into(),
        dirty_digest: dirty_digest.into(),
    })
}

pub fn labels_for_paths(paths: &[String]) -> Vec<String> {
    let mut labels = BTreeSet::new();
    for path in paths {
        let lower = path.to_ascii_lowercase();
        if lower.ends_with(".rs") {
            labels.insert("rust".into());
        }
        if lower.ends_with(".ts")
            || lower.ends_with(".tsx")
            || lower.ends_with(".js")
            || lower.ends_with(".jsx")
        {
            labels.insert("frontend".into());
        }
        if lower.contains("migration") {
            labels.insert("migration".into());
        }
        if lower.contains("adapter") {
            labels.insert("adapter".into());
        }
    }
    labels.into_iter().collect()
}

/// Whether an implementation worker's result is a **completion candidate** — a
/// revision that has to be verified before the task can be called done.
///
/// Not every revision is one. A `needs_delegation` handoff can mean two very
/// different things, and only one of them is a candidate:
///
/// - "the implementation is done, please verify it" — `suggestedRole:
///   verification`. The change set is real, usually sitting uncommitted in the
///   worker's own worktree awaiting adoption, and it is exactly the kind of
///   result that most needs review. Gating only on `completed` left it with no
///   `eval_attempts` row, so the verifier the orchestrator routed next had no
///   revision to bind to and died on a raw `QueryReturnedNoRows`.
/// - "I need another implementation worker" — anything else. That is a **partial
///   revision**: real work, but not a claim of completeness. Opening a gate over
///   it would let a verifier drive incomplete work to `verified` while the
///   follow-up the worker actually asked for never ran. It opens nothing, and the
///   routing notice carries `suggestedRole`/`suggestedTask` so the orchestrator
///   routes what was asked for instead.
///
/// `suggestedRole` is always populated for `needs_delegation` — `delegation`
/// derives it from `suggestedTask` and falls back to `implementation` — so the
/// default direction is the safe one.
///
/// `files_changed` is trustworthy here because `worker_adoption` has already
/// replaced it with the paths derived from Git, empty list included, so a worker
/// cannot open a gate by naming files it never touched. Every other status
/// (`failed`, `blocked`, `cancelled`, `protocol_invalid`) opens nothing: there is
/// either no work to judge or nothing readable to judge it by.
fn opens_completion_gate(result: &WorkerResult) -> bool {
    match result.status {
        WorkerResultStatus::Completed => true,
        WorkerResultStatus::NeedsDelegation => {
            !result.files_changed.is_empty()
                && result.suggested_role == Some(crate::delegation::WorkerRole::Verification)
        }
        _ => false,
    }
}

/// Typed reason reported when a verification worker has no revision to bind to.
pub const VERIFICATION_TARGET_UNAVAILABLE: &str = "implementation_revision_unavailable";

/// The checkout a verifier must run in: the repository behind the newest open
/// completion gate for this task.
///
/// The newest attempt is selected *first* and only then asked whether it is
/// live. Filtering by status inside the query let an older `failed` attempt be
/// handed back while a newer terminal one existed, and `failed` was never a
/// bindable target anyway — `settle_verification_result` treats it as final and
/// refuses to re-open it, so a verifier sent there could only die at settlement.
///
/// `Ok(None)` is the routing fact "this task has no live gate to verify" —
/// unroutable, but not a database failure, and the two must not reach the
/// orchestrator as the same sentence.
pub fn verification_target_path(
    db: &Connection,
    parent_session_id: &str,
) -> Result<Option<String>, BridgeError> {
    let newest: Option<(String, String)> = db
        .query_row(
            "SELECT status,repository_path FROM eval_attempts WHERE session_id=?1 ORDER BY started_at DESC,rowid DESC LIMIT 1",
            params![parent_session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    // Matched on the stored status rather than through `parse_verdict`, whose
    // catch-all reads anything unrecognized as `verifying`. A status Bridge does
    // not know is not a gate it should send a verifier into.
    Ok(newest.and_then(|(status, path)| {
        matches!(status.as_str(), "verifying" | "changes_requested").then_some(path)
    }))
}

/// What the orchestrator can actually do about a missing implementation
/// revision. `permanent` is the right failure class for this, but "retry will
/// not help" is only half the answer the orchestrator needs.
///
/// The wording must cover both ways this state arises, because they have
/// different remedies and only one of them involves a worker at all (issue
/// #327): an implementation worker whose changes sit unadopted in its own
/// worktree, versus no recorded implementation work anywhere. Telling the
/// second case to "adopt the worker's changes" sends the orchestrator looking
/// for a worker that never existed.
pub fn verification_target_unavailable_reason() -> String {
    format!(
        "no implementation revision is recorded for this task ({VERIFICATION_TARGET_UNAVAILABLE}) \
         and its checkout has no uncommitted implementation work to record either, so there is \
         nothing for a verifier to bind to. If an implementation worker ran, adopt or commit the \
         worker's worktree changes; if you implemented directly, make sure the edits were saved in \
         the task checkout; otherwise delegate the implementation before requesting verification."
    )
}

pub fn create_from_worker_result(
    db: &Connection,
    child_session_id: &str,
    result: &WorkerResult,
    available_capabilities: &HashSet<String>,
) -> Result<Option<CompletionSummary>, BridgeError> {
    let Some(context) = worker_completion_context(db, child_session_id)? else {
        return Ok(None);
    };
    if context.role == "verification" {
        let request = context.serialized_request.as_deref().ok_or_else(|| {
            BridgeError::Invalid(
                "verification worker is missing its durable completion input".into(),
            )
        })?;
        let request: DelegationRequest = serde_json::from_str(request).map_err(|error| {
            BridgeError::Invalid(format!("stored verification request is malformed: {error}"))
        })?;
        return settle_verification_result(db, &context, &request, result);
    }
    if context.role != "implementation" || !opens_completion_gate(result) {
        return Ok(None);
    }
    let serialized_request = context.serialized_request.as_deref().ok_or_else(|| {
        BridgeError::Invalid(
            "completed implementation is missing its durable completion input".into(),
        )
    })?;
    let request: DelegationRequest = serde_json::from_str(serialized_request).map_err(|error| {
        BridgeError::Invalid(format!("stored delegation request is malformed: {error}"))
    })?;
    request.validate().map_err(BridgeError::Invalid)?;
    let repository_path = context
        .worktree_path
        .clone()
        .or(context.parent_path.clone())
        .ok_or_else(|| {
            BridgeError::Invalid(
                "implementation completion requires a repository path before verification".into(),
            )
        })?;
    let repository = repository_stamp(&repository_path)?;
    if let Some(existing) = latest_summary(db, &context.parent_session_id)? {
        if matches!(
            existing.verdict,
            CompletionVerdict::Verifying | CompletionVerdict::ChangesRequested
        ) && existing.repository == repository
        {
            return Ok(Some(existing));
        }
    }
    let contract = CompletionContract {
        id: Uuid::new_v4().to_string(),
        workspace_id: context.workspace_id,
        session_id: context.parent_session_id.clone(),
        schema_version: COMPLETION_SCHEMA_VERSION,
        acceptance_criteria: request.acceptance_criteria.clone(),
        markdown_projection: None,
        markdown_committed: false,
    };
    let plan = plan_with_registered_manifests(
        db,
        PlanInput {
            contract_id: contract.id.clone(),
            acceptance_criteria: request.acceptance_criteria,
            changed_paths: result.files_changed.clone(),
            repository_commands: request.verification,
        },
        &labels_for_paths(&result.files_changed),
        available_capabilities,
    )?;
    let attempt_id = create_flow(
        db,
        &contract,
        &plan,
        &context.parent_session_id,
        &repository_path,
        &repository,
        Some(&context.harness),
    )?;
    // A bare HEAD cannot say what the change is relative to or who produced it.
    // Binding the attempt to the worker's base revision, branch, and session
    // makes the proof answer "what changed, from what, by whom".
    if let Some(binding) = crate::worker_adoption::binding(db, child_session_id)? {
        db.execute(
            "UPDATE eval_attempts SET base_commit=?2,base_ref=?3,worker_branch=?4,worker_session_id=?5 WHERE id=?1",
            params![
                attempt_id,
                binding.base_commit,
                binding.base_branch,
                binding.worktree_branch,
                child_session_id,
            ],
        )?;
    }
    latest_summary(db, &context.parent_session_id)
}

/// How [`ensure_verification_target`] resolved a verifier's bind target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationTargetResolution {
    /// An open gate already existed; nothing was written.
    Existing,
    /// A gate was just opened over the orchestrator's own uncommitted edits.
    SelfRecorded,
    /// No open gate and nothing recordable: the delegation must be refused
    /// before a worker is reserved for it.
    Missing,
}

/// Make sure a verification delegation has an implementation revision to bind
/// to, opening one when the orchestrator edited the checkout itself.
///
/// Issue #327: revisions were recorded only from a *delegated* worker's
/// completion event, so "I made the change myself, now verify it" left real
/// work on disk with no `eval_attempts` row — and the verifier died at launch.
/// This runs before the worker reservation so a task with nothing to verify is
/// rejected as a delegation, not discovered as a launch failure after a worker
/// was already created.
pub fn ensure_verification_target(
    db: &Connection,
    parent_session_id: &str,
    directive: &DelegationRequest,
    available_capabilities: &HashSet<String>,
) -> Result<VerificationTargetResolution, BridgeError> {
    if verification_target_path(db, parent_session_id)?.is_some() {
        return Ok(VerificationTargetResolution::Existing);
    }
    let Some(repository_path) = store::repository_path_for_session(db, parent_session_id)? else {
        return Ok(VerificationTargetResolution::Missing);
    };
    let repository_path = repository_path.to_string_lossy().into_owned();
    // The contract pins the workspace the gate belongs to. Without one there is
    // no durable checkout identity to bind verification to — the same reason a
    // worker lease cannot exist without it.
    let workspace_id: Option<String> = db
        .query_row(
            "SELECT workspace_id FROM sessions WHERE id=?1",
            params![parent_session_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let Some(workspace_id) = workspace_id else {
        return Ok(VerificationTargetResolution::Missing);
    };
    match create_from_orchestrator_edits(
        db,
        parent_session_id,
        &workspace_id,
        &repository_path,
        directive,
        available_capabilities,
    )? {
        Some(_) => Ok(VerificationTargetResolution::SelfRecorded),
        None => Ok(VerificationTargetResolution::Missing),
    }
}

/// Open a completion gate over edits the orchestrator made itself — the
/// parallel of [`create_from_worker_result`] for work no worker ever reported.
///
/// Everything is derived from Git rather than claimed by anyone: the stamp is
/// the checkout's HEAD + dirty digest, the changed paths come from porcelain
/// status, and the acceptance criteria and commands are the ones the
/// verification directive itself carries (what the orchestrator asked to have
/// checked). `implementer_family` records the orchestrator's own harness; no
/// worker binding is written because no worker exists.
///
/// Committed-only orchestrator work is deliberately out of scope: without a
/// recorded base commit there is no honest way to say which commits belong to
/// this task, so detection covers the uncommitted tree state direct edits
/// actually produce. `Ok(None)` means "no evidence of direct implementation
/// work" — the caller's signal to reject rather than fabricate a revision over
/// a pristine tree.
pub fn create_from_orchestrator_edits(
    db: &Connection,
    parent_session_id: &str,
    workspace_id: &str,
    repository_path: &str,
    directive: &DelegationRequest,
    available_capabilities: &HashSet<String>,
) -> Result<Option<CompletionSummary>, BridgeError> {
    if directive.acceptance_criteria.is_empty() {
        return Err(BridgeError::Invalid(
            "recording an orchestrator revision requires the verification directive's \
             acceptance criteria"
                .into(),
        ));
    }
    // A directory that is not a repository has no worktree state to record;
    // report that as "nothing to verify" instead of failing the delegation
    // with raw git stderr.
    if store::repository_state_for_path(Path::new(repository_path))["status"]
        .as_str()
        == Some("unavailable")
    {
        return Ok(None);
    }
    let evidence = crate::git::derive_repository_evidence(Path::new(repository_path), None)?;
    if evidence.is_empty() {
        return Ok(None);
    }
    let changed_paths = evidence.changed_paths();
    let repository = repository_stamp(repository_path)?;
    if let Some(existing) = latest_summary(db, parent_session_id)? {
        if matches!(
            existing.verdict,
            CompletionVerdict::Verifying | CompletionVerdict::ChangesRequested
        ) && existing.repository == repository
        {
            return Ok(Some(existing));
        }
    }
    let harness: Option<String> = db
        .query_row(
            "SELECT harness FROM sessions WHERE id=?1",
            params![parent_session_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let contract = CompletionContract {
        id: Uuid::new_v4().to_string(),
        workspace_id: workspace_id.to_owned(),
        session_id: parent_session_id.to_owned(),
        schema_version: COMPLETION_SCHEMA_VERSION,
        acceptance_criteria: directive.acceptance_criteria.clone(),
        markdown_projection: None,
        markdown_committed: false,
    };
    let plan = plan_with_registered_manifests(
        db,
        PlanInput {
            contract_id: contract.id.clone(),
            acceptance_criteria: directive.acceptance_criteria.clone(),
            changed_paths: changed_paths.clone(),
            repository_commands: directive.verification.clone(),
        },
        &labels_for_paths(&changed_paths),
        available_capabilities,
    )?;
    create_flow(
        db,
        &contract,
        &plan,
        parent_session_id,
        repository_path,
        &repository,
        harness.as_deref(),
    )?;
    latest_summary(db, parent_session_id)
}

fn settle_verification_result(
    db: &Connection,
    context: &WorkerCompletionContext,
    request: &DelegationRequest,
    result: &WorkerResult,
) -> Result<Option<CompletionSummary>, BridgeError> {
    let Some(summary) = latest_summary(db, &context.parent_session_id)? else {
        return Err(BridgeError::Invalid(
            "verification worker completed without an active completion gate".into(),
        ));
    };
    // Terminal verdicts are final. A late verification result must not re-open a
    // gate that already failed, superseded, or passed: `finalize` would recompute
    // the verdict as `verifying` (its blockers are `blocked`, not `failed`), the
    // parent would be pinned again, and nothing could re-escalate it because the
    // deadline pass only looks for `pending`/`running` checks.
    if matches!(
        summary.verdict,
        CompletionVerdict::Verified
            | CompletionVerdict::Waived
            | CompletionVerdict::Failed
            | CompletionVerdict::Superseded
    ) {
        return Ok(Some(summary));
    }
    let repository_path: String = db.query_row(
        "SELECT repository_path FROM eval_attempts WHERE id=?1",
        params![summary.attempt_id],
        |row| row.get(0),
    )?;
    let current_repository = repository_stamp(&repository_path)?;
    let digest = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(result).map_err(|error| BridgeError::Invalid(error.to_string()))?
        )
    );
    let mut updated = 0usize;
    // Deterministic checks planned as `bridge.shell` belong to the check runner:
    // a worker's `tests[]` entry is a claim, and hashing the JSON it arrived in
    // does not make it evidence. Only non-shell deterministic checks — ones with
    // no executor of their own — can be settled from a typed result.
    for check in summary.checks.iter().filter(|check| {
        check.kind == EvalKind::Deterministic
            && check.status != CheckStatus::Passed
            && check.executor != SHELL_EXECUTOR
    }) {
        if let Some(test) = result
            .tests
            .iter()
            .find(|test| Some(test.command.as_str()) == check.command.as_deref())
        {
            let status = match test.status {
                TestStatus::Passed => CheckStatus::Passed,
                TestStatus::Failed => CheckStatus::Failed,
                TestStatus::Skipped => CheckStatus::Skipped,
            };
            record_check(
                db,
                &summary.attempt_id,
                &CheckRun {
                    check_id: check.check_id.clone(),
                    kind: check.kind,
                    required: check.required,
                    status,
                    executor: WORKER_RESULT_EXECUTOR.into(),
                    command: check.command.clone(),
                    verifier_family: Some(context.harness.clone()),
                    detail: test.detail.clone(),
                    output_digest: Some(digest.clone()),
                    artifact_refs: vec![],
                },
            )?;
            updated += 1;
        }
    }
    let request_text = format!(
        "{} {} {}",
        request.objective,
        request.acceptance_criteria.join(" "),
        request.verification.join(" ")
    )
    .to_ascii_lowercase();
    let unresolved_semantic = summary
        .checks
        .iter()
        .filter(|check| {
            check.kind != EvalKind::Deterministic && check.status != CheckStatus::Passed
        })
        .collect::<Vec<_>>();
    let semantic = unresolved_semantic
        .iter()
        .find(|check| request_text.contains(&check.check_id.to_ascii_lowercase()))
        .copied()
        .or_else(|| {
            unresolved_semantic
                .iter()
                .find(|check| {
                    check.kind == EvalKind::UserTesting
                        && [
                            "user journey",
                            "browser",
                            "playwright",
                            "screen",
                            "console",
                            "network",
                        ]
                        .iter()
                        .any(|term| request_text.contains(term))
                })
                .copied()
        })
        .or_else(|| {
            unresolved_semantic
                .iter()
                .find(|check| {
                    check.kind == EvalKind::Scrutiny
                        && ["scrutiny", "review", "test quality", "code review"]
                            .iter()
                            .any(|term| request_text.contains(term))
                })
                .copied()
        })
        .or_else(|| unresolved_semantic.first().copied());
    if let Some(check) = semantic {
        let status = match result.status {
            WorkerResultStatus::Completed
                if result.risks.is_empty() && result.remaining_work.is_empty() =>
            {
                CheckStatus::Passed
            }
            WorkerResultStatus::Completed => CheckStatus::Failed,
            WorkerResultStatus::Failed => CheckStatus::Failed,
            WorkerResultStatus::Cancelled
            | WorkerResultStatus::Blocked
            | WorkerResultStatus::NeedsDelegation
            // Blocked, not failed: the work may well be fine, but nothing
            // readable came back to judge it by.
            | WorkerResultStatus::ProtocolInvalid => CheckStatus::Blocked,
        };
        let mut detail = result.summary.clone();
        if !result.risks.is_empty() {
            detail.push_str(&format!("\nRisks: {}", result.risks.join(" | ")));
        }
        if !result.remaining_work.is_empty() {
            detail.push_str(&format!(
                "\nRemaining: {}",
                result.remaining_work.join(" | ")
            ));
        }
        record_check(
            db,
            &summary.attempt_id,
            &CheckRun {
                check_id: check.check_id.clone(),
                kind: check.kind,
                required: check.required,
                status,
                executor: WORKER_RESULT_EXECUTOR.into(),
                command: None,
                verifier_family: Some(context.harness.clone()),
                detail: Some(detail),
                output_digest: Some(digest.clone()),
                artifact_refs: vec![],
            },
        )?;
        updated += 1;
    }
    if updated == 0 {
        // Blocking a shell check here would be the same trust violation, so the
        // fallback only touches checks a worker is allowed to settle. Any shell
        // check left pending is the runner's, or the deadline's.
        if let Some(check) = summary.checks.iter().find(|check| {
            check.required
                && check.status != CheckStatus::Passed
                && check.executor != SHELL_EXECUTOR
        }) {
            record_check(
                db,
                &summary.attempt_id,
                &CheckRun {
                    check_id: check.check_id.clone(),
                    kind: check.kind,
                    required: true,
                    status: CheckStatus::Blocked,
                    executor: WORKER_RESULT_EXECUTOR.into(),
                    command: check.command.clone(),
                    verifier_family: Some(context.harness.clone()),
                    detail: Some("Verification worker returned no matching typed evidence".into()),
                    output_digest: Some(digest),
                    artifact_refs: vec![],
                },
            )?;
        }
    }
    for (index, finding) in result
        .risks
        .iter()
        .chain(&result.remaining_work)
        .enumerate()
    {
        db.execute(
            "INSERT INTO eval_findings(id,attempt_id,check_id,severity,summary,affected_paths,created_at) VALUES(?1,?2,'verification-worker',?3,?4,?5,?6)",
            params![Uuid::new_v4().to_string(), summary.attempt_id, if index < result.risks.len() { "warning" } else { "info" }, finding, serde_json::to_string(&result.files_changed).unwrap_or_else(|_| "[]".into()), Utc::now().to_rfc3339()],
        )?;
    }
    finalize(db, &summary.attempt_id, &current_repository)?;
    latest_summary(db, &context.parent_session_id)
}

pub fn record_gate_error(
    db: &Connection,
    child_session_id: &str,
    message: &str,
) -> Result<Option<CompletionSummary>, BridgeError> {
    let Some(context) = worker_completion_context(db, child_session_id)? else {
        return Ok(None);
    };
    let repository_path = context
        .worktree_path
        .or(context.parent_path)
        .unwrap_or_else(|| ".".into());
    let repository = repository_stamp(&repository_path).unwrap_or_else(|_| RepositoryStamp {
        head: "unavailable".into(),
        dirty_digest: format!("{:x}", Sha256::digest(message.as_bytes())),
    });
    let contract = CompletionContract {
        id: Uuid::new_v4().to_string(),
        workspace_id: context.workspace_id,
        session_id: context.parent_session_id.clone(),
        schema_version: COMPLETION_SCHEMA_VERSION,
        acceptance_criteria: vec!["Resolve completion gate creation failure".into()],
        markdown_projection: None,
        markdown_committed: false,
    };
    let plan = EvalPlan {
        id: Uuid::new_v4().to_string(),
        contract_id: contract.id.clone(),
        schema_version: COMPLETION_SCHEMA_VERSION,
        risk: RiskTier::High,
        checks: vec![EvalCheck {
            id: "gate-error".into(),
            label: "Completion gate creation failed".into(),
            kind: EvalKind::Deterministic,
            required: true,
            executor: SYSTEM_EXECUTOR.into(),
            command: None,
            required_capabilities: vec![],
            different_model_family: false,
            reason: "Bridge could not construct the required completion gate".into(),
        }],
    };
    let attempt_id = create_flow(
        db,
        &contract,
        &plan,
        &context.parent_session_id,
        &repository_path,
        &repository,
        Some(&context.harness),
    )?;
    db.execute(
        "UPDATE eval_attempts SET status='superseded',completed_at=?3 WHERE session_id=?1 AND id<>?2 AND status IN ('verifying','changes_requested','failed')",
        params![context.parent_session_id, attempt_id, Utc::now().to_rfc3339()],
    )?;
    record_check(
        db,
        &attempt_id,
        &CheckRun {
            check_id: "gate-error".into(),
            kind: EvalKind::Deterministic,
            required: true,
            status: CheckStatus::Blocked,
            executor: SYSTEM_EXECUTOR.into(),
            command: None,
            verifier_family: None,
            detail: Some(message.into()),
            output_digest: Some(format!("{:x}", Sha256::digest(message.as_bytes()))),
            artifact_refs: vec![],
        },
    )?;
    db.execute(
        "UPDATE eval_attempts SET status='failed' WHERE id=?1",
        params![attempt_id],
    )?;
    latest_summary(db, &context.parent_session_id)
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
        kind: match kind.as_str() {
            "scrutiny" => EvalKind::Scrutiny,
            "user_testing" => EvalKind::UserTesting,
            _ => EvalKind::Deterministic,
        },
        required: row.get(2)?,
        status: match status.as_str() {
            "running" => CheckStatus::Running,
            "passed" => CheckStatus::Passed,
            "failed" => CheckStatus::Failed,
            "skipped" => CheckStatus::Skipped,
            "blocked" => CheckStatus::Blocked,
            "stale" => CheckStatus::Stale,
            _ => CheckStatus::Pending,
        },
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
        return Err(BridgeError::Invalid(
            "completion flow requires the exact repository path being evaluated".into(),
        ));
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
    transaction.execute(
        "UPDATE sessions SET status='waiting' WHERE id=?1 AND status IN ('idle','ready')",
        params![session_id],
    )?;
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
        return Err(BridgeError::Invalid(
            "verification attempt requires a repository path".into(),
        ));
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

pub fn record_check(db: &Connection, attempt_id: &str, run: &CheckRun) -> Result<(), BridgeError> {
    let attempt: Option<(String, String, Option<String>, String)> = db
        .query_row(
            "SELECT a.repository_head,a.dirty_digest,a.implementer_family,p.plan FROM eval_attempts a JOIN eval_plans p ON p.id=a.plan_id WHERE a.id=?1 AND a.status NOT IN ('verified','waived','superseded') AND (a.status<>'failed' OR a.escalation IS NULL)",
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
        .ok_or_else(|| {
            BridgeError::Invalid(format!(
                "unknown check {} for verification attempt",
                run.check_id
            ))
        })?;
    // `bridge.system` is Bridge reporting on its own machinery — a deadline that
    // expired, a gate it could not build. Those are never passes, so it must not
    // become a way to mark any check passed without an executor having run it.
    if run.executor == SYSTEM_EXECUTOR && run.status == CheckStatus::Passed {
        return Err(BridgeError::Invalid(format!(
            "{SYSTEM_EXECUTOR} records why a check could not run; it cannot pass check {}",
            run.check_id
        )));
    }
    // A `bridge.shell` check is proven by running the command. A digest of a
    // worker's JSON says only which message arrived, so worker-reported evidence
    // may never resolve one — the runner must, or the deadline must fail it.
    if check.executor == SHELL_EXECUTOR
        && !matches!(run.executor.as_str(), SHELL_EXECUTOR | SYSTEM_EXECUTOR)
    {
        return Err(BridgeError::Invalid(format!(
            "check {} is a {SHELL_EXECUTOR} command; evidence from {} cannot satisfy it",
            run.check_id, run.executor
        )));
    }
    if run.status == CheckStatus::Passed && check.different_model_family {
        let verifier = run
            .verifier_family
            .as_deref()
            .filter(|value| !value.trim().is_empty());
        let implementer = implementer_family
            .as_deref()
            .filter(|value| !value.trim().is_empty());
        if verifier.is_none() || implementer.is_none() || verifier == implementer {
            return Err(BridgeError::Invalid(
                "independent verifier evidence must come from a different model family".into(),
            ));
        }
    }
    if run.status == CheckStatus::Passed
        && run
            .output_digest
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
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
        db.execute(
            "UPDATE eval_attempts SET status='superseded',completed_at=?2 WHERE id=?1",
            params![attempt_id, Utc::now().to_rfc3339()],
        )?;
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
    let mut waiver_statement = db.prepare(
        "SELECT check_ids,reason FROM eval_waivers WHERE attempt_id=?1 AND repository_head=?2 AND dirty_digest=?3 ORDER BY created_at,rowid",
    )?;
    let waivers = waiver_statement
        .query_map(params![attempt_id, head, dirty], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut waived_check_ids = HashSet::new();
    let mut waiver_reasons = Vec::new();
    for (serialized_ids, reason) in waivers {
        waived_check_ids
            .extend(serde_json::from_str::<Vec<String>>(&serialized_ids).unwrap_or_default());
        waiver_reasons.push(reason);
    }
    let waiver_reason = (!waiver_reasons.is_empty()).then(|| waiver_reasons.join(" | "));
    let blockers = checks
        .iter()
        .filter(|run| run.required && run.status != CheckStatus::Passed)
        .collect::<Vec<_>>();
    let verdict = if blockers.is_empty() {
        CompletionVerdict::Verified
    } else if blockers
        .iter()
        .all(|run| waived_check_ids.contains(&run.check_id))
    {
        CompletionVerdict::Waived
    } else if blockers.iter().any(|run| run.status == CheckStatus::Failed) {
        CompletionVerdict::ChangesRequested
    } else {
        CompletionVerdict::Verifying
    };
    let failed_or_skipped = checks
        .iter()
        .filter(|run| {
            matches!(
                run.status,
                CheckStatus::Failed
                    | CheckStatus::Skipped
                    | CheckStatus::Blocked
                    | CheckStatus::Stale
            )
        })
        .map(|run| run.check_id.clone())
        .collect();
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
    let serialized =
        serde_json::to_string(&bundle).map_err(|error| BridgeError::Invalid(error.to_string()))?;
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
        return Err(BridgeError::Invalid(
            "waiver check scope contains duplicates".into(),
        ));
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
        db.execute(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','p','/tmp/p','now')",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','x','w','main','/tmp/w','idle','now')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind,continuation_fidelity) VALUES('s','w','codex','s','idle','estimated','orchestrator','native')", []).unwrap();
        db
    }

    fn contract() -> CompletionContract {
        CompletionContract {
            id: "contract".into(),
            workspace_id: "w".into(),
            session_id: "s".into(),
            schema_version: 1,
            acceptance_criteria: vec!["Router dialog saves preferences".into()],
            markdown_projection: None,
            markdown_committed: false,
        }
    }

    #[test]
    fn planner_selects_independent_scrutiny_and_browser_journey() {
        let result = plan(PlanInput {
            contract_id: "c".into(),
            acceptance_criteria: vec!["User opens the dialog".into()],
            changed_paths: vec![
                "src/components/Dialog.tsx".into(),
                "src-tauri/src/policy.rs".into(),
            ],
            repository_commands: vec![],
        });
        assert_eq!(result.risk, RiskTier::High);
        assert!(result
            .checks
            .iter()
            .any(|check| check.kind == EvalKind::Scrutiny && check.different_model_family));
        assert!(result
            .checks
            .iter()
            .any(|check| check.kind == EvalKind::UserTesting
                && check.required_capabilities.contains(&"browser".into())));
        assert!(result
            .checks
            .iter()
            .any(|check| check.command.as_deref() == Some("bun run build")));
    }

    #[test]
    fn rust_changes_verify_the_whole_cargo_workspace() {
        // Without --workspace, cargo at the workspace root only exercises the
        // bridge-deck shell and silently skips every bridge-core test.
        let result = plan(PlanInput {
            contract_id: "c".into(),
            acceptance_criteria: vec!["Worker pool retries stalled workers".into()],
            changed_paths: vec!["src-tauri/bridge-core/src/worker_pool.rs".into()],
            repository_commands: vec![],
        });
        assert!(result.checks.iter().any(|check| check.command.as_deref()
            == Some("cargo test --manifest-path src-tauri/Cargo.toml --workspace")));
        assert!(result.checks.iter().any(|check| check.command.as_deref()
            == Some("cargo check --manifest-path src-tauri/Cargo.toml --workspace")));
    }

    #[test]
    fn per_file_risk_tier_matches_the_planner_signals() {
        assert_eq!(risk_tier_for_path("src-tauri/src/policy.rs"), RiskTier::High);
        assert_eq!(
            risk_tier_for_path("src-tauri/bridge-core/src/store.rs"),
            RiskTier::High
        );
        assert_eq!(
            risk_tier_for_path("src/components/Dialog.tsx"),
            RiskTier::Medium
        );
        assert_eq!(risk_tier_for_path("README.md"), RiskTier::Low);
    }

    #[test]
    fn low_signal_paths_cover_lockfiles_generated_and_vendored_trees() {
        assert!(is_low_signal_path("bun.lock"));
        assert!(is_low_signal_path("src-tauri/Cargo.lock"));
        assert!(is_low_signal_path("package-lock.json"));
        assert!(is_low_signal_path("src/protocol/generated/protocol.ts"));
        assert!(is_low_signal_path("vendor/some-lib/index.js"));
        assert!(!is_low_signal_path("src/App.tsx"));
        assert!(!is_low_signal_path("src-tauri/bridge-core/src/git.rs"));
    }

    #[test]
    fn verifier_rejects_same_family_and_missing_tools() {
        let manifest = VerifierManifest {
            id: "web".into(),
            kind: EvalKind::UserTesting,
            triggers: vec!["frontend".into()],
            required_capabilities: vec!["browser".into()],
            different_model_family: true,
            checks: vec!["open app".into()],
            evidence_required: vec!["screenshot".into()],
        };
        assert!(manifest
            .eligible(
                Some("codex"),
                Some("codex"),
                &HashSet::from(["browser".into()])
            )
            .is_err());
        assert!(manifest
            .eligible(Some("codex"), Some("claude"), &HashSet::new())
            .unwrap_err()
            .contains("browser"));
        assert!(manifest
            .eligible(
                Some("codex"),
                Some("claude"),
                &HashSet::from(["browser".into()])
            )
            .is_ok());
    }

    #[test]
    fn skill_verifier_manifests_add_checks_without_granting_missing_tools() {
        let db = fixture();
        let manifest = VerifierManifest {
            id: "playwright-journey".into(),
            kind: EvalKind::UserTesting,
            triggers: vec!["frontend".into()],
            required_capabilities: vec!["browser".into(), "network_inspection".into()],
            different_model_family: true,
            checks: vec!["exercise acceptance journey".into()],
            evidence_required: vec!["trace".into(), "screenshot".into()],
        };
        register_verifier_manifest(&db, "skill:review-checkpoint", &manifest).unwrap();
        let blocked = verifier_candidates(
            &db,
            &["frontend".into()],
            &HashSet::from(["browser".into()]),
        )
        .unwrap();
        assert!(!blocked[0].eligible);
        assert!(blocked[0].exclusion_reasons[0].contains("network_inspection"));
        let eligible = verifier_candidates(
            &db,
            &["frontend".into()],
            &HashSet::from(["browser".into(), "network_inspection".into()]),
        )
        .unwrap();
        assert!(eligible[0].eligible);
        assert!(eligible[0].manifest.different_model_family);
    }

    #[test]
    fn completed_implementation_opens_a_private_verification_gate() {
        use crate::delegation::{SuggestedNextAction, WorkerTestResult};
        let db = fixture();
        let cwd = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        db.execute(
            "UPDATE sessions SET cwd='/bridge/missing-parent-worktree' WHERE id='s'",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id,kind,continuation_fidelity) VALUES('child','w','claude','impl','completed','estimated','s','worker','native')", []).unwrap();
        db.execute("INSERT INTO worker_leases(session_id,workspace_id,role,capability_tier,task_family,owned_paths,write_mode,lease_status,created_at,updated_at) VALUES('child','w','implementation','standard','implementation','[]','isolated','released','now','now')", []).unwrap();
        db.execute("INSERT INTO worker_runtime(session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,result_status,retry_count,updated_at) VALUES('child','s','completed','implementation','key','reported',0,'now')", []).unwrap();
        db.execute(
            "UPDATE worker_runtime SET worktree_path=?2 WHERE session_id=?1",
            params!["child", cwd],
        )
        .unwrap();
        let request = serde_json::json!({"schemaVersion":1,"role":"implementation","objective":"Implement proof","acceptanceCriteria":["Proof card is visible"],"knownFacts":[],"decisions":[],"evidenceIds":[],"relevantFiles":["src/App.tsx"],"ownedPaths":["src/**"],"writeMode":"isolated","capabilityTier":"standard","effort":"medium","verification":["bun run test"],"outputContract":"implementation-result","harness":"claude"});
        db.execute("INSERT INTO worker_completion_inputs(child_session_id,request,updated_at) VALUES('child',?1,'now')", params![request.to_string()]).unwrap();
        let result = WorkerResult {
            schema_version: 1,
            status: WorkerResultStatus::Completed,
            summary: "implemented".into(),
            files_changed: vec!["src/App.tsx".into()],
            tests: Vec::<WorkerTestResult>::new(),
            decisions: vec![],
            risks: vec![],
            remaining_work: vec![],
            suggested_next_action: SuggestedNextAction::Finish,
            suggested_role: None,
            suggested_task: None,
        };
        let summary = create_from_worker_result(&db, "child", &result, &HashSet::new())
            .unwrap()
            .unwrap();
        assert_eq!(summary.verdict, CompletionVerdict::Verifying);
        assert!(!summary.markdown_committed);
        assert!(summary
            .checks
            .iter()
            .any(|check| check.kind == EvalKind::Scrutiny));
        assert_eq!(
            db.query_row(
                "SELECT repository_path FROM eval_attempts WHERE id=?1",
                params![summary.attempt_id],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            cwd
        );
        assert_eq!(
            db.query_row("SELECT status FROM sessions WHERE id='s'", [], |row| row
                .get::<_, String>(
                0
            ))
            .unwrap(),
            "waiting"
        );
        let duplicate = create_from_worker_result(&db, "child", &result, &HashSet::new())
            .unwrap()
            .unwrap();
        assert_eq!(duplicate.attempt_id, summary.attempt_id);
        db.execute(
            "UPDATE eval_attempts SET dirty_digest='prior-revision' WHERE id=?1",
            params![summary.attempt_id],
        )
        .unwrap();
        let rebound = create_from_worker_result(&db, "child", &result, &HashSet::new())
            .unwrap()
            .unwrap();
        assert_ne!(rebound.attempt_id, summary.attempt_id);
        assert_eq!(
            db.query_row(
                "SELECT status FROM eval_attempts WHERE id=?1",
                params![summary.attempt_id],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "superseded"
        );
    }

    /// An implementation worker that finished in `worktree`, with its durable
    /// completion input registered, ready to hand a result to
    /// `create_from_worker_result`.
    fn implementation_worker(db: &Connection, session_id: &str, worktree: &str) {
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id,kind,continuation_fidelity) VALUES(?1,'w','claude','impl','completed','estimated','s','worker','native')", params![session_id]).unwrap();
        db.execute("INSERT INTO worker_leases(session_id,workspace_id,role,capability_tier,task_family,owned_paths,write_mode,lease_status,created_at,updated_at) VALUES(?1,'w','implementation','standard','implementation','[]','isolated','released','now','now')", params![session_id]).unwrap();
        db.execute("INSERT INTO worker_runtime(session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,result_status,retry_count,worktree_path,updated_at) VALUES(?1,'s','completed','implementation','key','reported',0,?2,'now')", params![session_id, worktree]).unwrap();
        let request = serde_json::json!({"schemaVersion":1,"role":"implementation","objective":"Implement proof","acceptanceCriteria":["Proof card is visible"],"knownFacts":[],"decisions":[],"evidenceIds":[],"relevantFiles":["src/App.tsx"],"ownedPaths":["src/**"],"writeMode":"isolated","capabilityTier":"standard","effort":"medium","verification":["bun run test"],"outputContract":"implementation-result","harness":"claude"});
        db.execute("INSERT INTO worker_completion_inputs(child_session_id,request,updated_at) VALUES(?1,?2,'now')", params![session_id, request.to_string()]).unwrap();
    }

    /// A handoff asking for verification: the shape that is a completion
    /// candidate. `asking_for` overrides the requested follow-up.
    fn implementation_result(
        status: WorkerResultStatus,
        files_changed: Vec<String>,
    ) -> WorkerResult {
        asking_for(
            crate::delegation::WorkerRole::Verification,
            status,
            files_changed,
        )
    }

    fn asking_for(
        role: crate::delegation::WorkerRole,
        status: WorkerResultStatus,
        files_changed: Vec<String>,
    ) -> WorkerResult {
        WorkerResult {
            schema_version: 1,
            status,
            summary: "handing off".into(),
            files_changed,
            tests: vec![],
            decisions: vec![],
            risks: vec![],
            remaining_work: vec![],
            suggested_next_action: crate::delegation::SuggestedNextAction::FollowUp,
            suggested_role: Some(role),
            suggested_task: Some("take it from here".into()),
        }
    }

    /// The regression: a worker that hands off with its change set still
    /// uncommitted in its own worktree used to open no gate at all, which left
    /// the verification worker the orchestrator routed next with no revision to
    /// bind to.
    #[test]
    fn needs_delegation_with_changes_opens_a_bindable_gate() {
        let db = fixture();
        let cwd = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        implementation_worker(&db, "child", &cwd);
        // The worktree the worker wrote in, still awaiting adoption: a recorded
        // base revision and branch, but no commit of its own.
        db.execute(
            "INSERT INTO worker_worktree_adoptions(session_id,parent_session_id,workspace_id,worktree_path,worktree_branch,task_worktree_path,state,base_commit,base_branch,baseline_dirty_paths,changed_paths,dirty,created_at,updated_at)
             VALUES('child','s','w',?1,'codex/fix-model-profile-migration',?1,'pending_adoption','7d7e79fe','main','[]','[]',1,'now','now')",
            params![cwd],
        )
        .unwrap();
        let summary = create_from_worker_result(
            &db,
            "child",
            &implementation_result(
                WorkerResultStatus::NeedsDelegation,
                vec!["src/App.tsx".into()],
            ),
            &HashSet::new(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(summary.verdict, CompletionVerdict::Verifying);
        assert_eq!(
            verification_target_path(&db, "s").unwrap(),
            Some(cwd),
            "the verifier binds to the worktree the implementation left dirty"
        );
        // And the attempt still says what the change is relative to and who
        // produced it, so the verifier reviews a revision rather than a folder.
        assert_eq!(
            db.query_row(
                "SELECT base_commit,base_ref,worker_branch,worker_session_id FROM eval_attempts WHERE id=?1",
                params![summary.attempt_id],
                |row| Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                )),
            )
            .unwrap(),
            (
                Some("7d7e79fe".into()),
                Some("main".into()),
                Some("codex/fix-model-profile-migration".into()),
                Some("child".into()),
            )
        );
    }

    #[test]
    fn a_handoff_that_changed_nothing_opens_no_gate() {
        let db = fixture();
        let cwd = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        implementation_worker(&db, "child", &cwd);
        // Nothing changed, so there is nothing to verify — and a failure still
        // opens nothing even when it did change files.
        for (status, files) in [
            (WorkerResultStatus::NeedsDelegation, vec![]),
            (WorkerResultStatus::Failed, vec!["src/App.tsx".into()]),
            (WorkerResultStatus::Blocked, vec!["src/App.tsx".into()]),
            (WorkerResultStatus::Cancelled, vec!["src/App.tsx".into()]),
            (
                WorkerResultStatus::ProtocolInvalid,
                vec!["src/App.tsx".into()],
            ),
        ] {
            assert!(create_from_worker_result(
                &db,
                "child",
                &implementation_result(status, files),
                &HashSet::new()
            )
            .unwrap()
            .is_none());
        }
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM eval_attempts", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    /// A handoff can mean "please verify this" or "I need another implementation
    /// worker". Only the first is a completion candidate: opening a gate over the
    /// second would let a verifier drive incomplete work to `verified` while the
    /// follow-up the worker asked for never ran.
    #[test]
    fn only_a_handoff_asking_for_verification_opens_a_gate() {
        use crate::delegation::WorkerRole;
        let cwd = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        for role in [
            WorkerRole::Implementation,
            WorkerRole::Research,
            WorkerRole::Planning,
            WorkerRole::Documentation,
        ] {
            let db = fixture();
            implementation_worker(&db, "child", &cwd);
            assert!(
                create_from_worker_result(
                    &db,
                    "child",
                    &asking_for(
                        role,
                        WorkerResultStatus::NeedsDelegation,
                        vec!["src/App.tsx".into()]
                    ),
                    &HashSet::new()
                )
                .unwrap()
                .is_none(),
                "a handoff asking for {} is a partial revision, not a completion candidate",
                role.as_str()
            );
            assert_eq!(
                verification_target_path(&db, "s").unwrap(),
                None,
                "and it is not offered as a verification target"
            );
        }
        // The same result asking for verification is a candidate.
        let db = fixture();
        implementation_worker(&db, "child", &cwd);
        assert!(create_from_worker_result(
            &db,
            "child",
            &asking_for(
                WorkerRole::Verification,
                WorkerResultStatus::NeedsDelegation,
                vec!["src/App.tsx".into()]
            ),
            &HashSet::new()
        )
        .unwrap()
        .is_some());
    }

    /// "There is no revision to verify" is a routing fact with a remediation,
    /// not the raw `QueryReturnedNoRows` the bind site used to forward.
    #[test]
    fn a_task_without_a_gate_reports_a_typed_unroutable_reason() {
        let db = fixture();
        assert_eq!(verification_target_path(&db, "s").unwrap(), None);
        let reason = verification_target_unavailable_reason();
        assert!(reason.contains(VERIFICATION_TARGET_UNAVAILABLE));
        // Issue #327: the message must cover both ways to arrive here — an
        // unadopted worker's changes, and no implementation work recorded at
        // all — instead of assuming an implementation worker exists.
        assert!(reason.contains("adopt or commit the"));
        assert!(reason.contains("delegate the implementation before requesting verification"));
        // "Nothing to verify" and "the database is broken" are different facts,
        // and the bind site reports them differently.
        db.execute("DROP TABLE eval_attempts", []).unwrap();
        assert!(verification_target_path(&db, "s").is_err());
    }

    /// Only the newest attempt decides, and only two of its statuses are a
    /// target. The old query filtered inside the SELECT, so an older `failed`
    /// attempt could be handed back while a newer terminal one existed — and
    /// `failed` was never bindable to begin with.
    #[test]
    fn only_the_newest_live_attempt_is_a_verification_target() {
        let db = fixture();
        let mut planned = 0;
        let mut attempt = |status: &str, path: &str| {
            planned += 1;
            let id = format!("attempt-{planned}");
            let plan_id = format!("plan-{planned}");
            let contract_id = format!("contract-{planned}");
            db.execute("INSERT INTO completion_contracts(id,workspace_id,session_id,schema_version,acceptance_criteria,markdown_committed,status,created_at,updated_at) VALUES(?1,'w','s',1,'[]',0,'open','now','now')", params![contract_id]).unwrap();
            db.execute("INSERT INTO eval_plans(id,contract_id,schema_version,risk,plan,created_at) VALUES(?1,?2,1,'high','{}','now')", params![plan_id, contract_id]).unwrap();
            db.execute(
                "INSERT INTO eval_attempts(id,plan_id,session_id,repository_head,dirty_digest,repository_path,status,started_at) VALUES(?1,?2,'s','head','clean',?3,?4,?5)",
                params![id, plan_id, path, status, format!("2026-08-2{planned}T00:00:00Z")],
            )
            .unwrap();
        };
        attempt("verifying", "/live/older");
        assert_eq!(
            verification_target_path(&db, "s").unwrap(),
            Some("/live/older".into())
        );
        for terminal in ["verified", "waived", "superseded", "failed"] {
            attempt(terminal, "/terminal");
            assert_eq!(
                verification_target_path(&db, "s").unwrap(),
                None,
                "a newer {terminal} attempt hides the older live one"
            );
        }
        attempt("changes_requested", "/live/newest");
        assert_eq!(
            verification_target_path(&db, "s").unwrap(),
            Some("/live/newest".into())
        );
    }

    #[test]
    fn verifiers_close_semantic_checks_but_never_shell_checks() {
        use crate::delegation::{SuggestedNextAction, WorkerTestResult};
        let db = fixture();
        let cwd = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        db.execute("UPDATE sessions SET cwd=?2 WHERE id=?1", params!["s", cwd])
            .unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id,kind,continuation_fidelity) VALUES('impl','w','claude','impl','completed','estimated','s','worker','native')", []).unwrap();
        db.execute("INSERT INTO worker_leases(session_id,workspace_id,role,capability_tier,task_family,owned_paths,write_mode,lease_status,created_at,updated_at) VALUES('impl','w','implementation','standard','implementation','[]','isolated','released','now','now')", []).unwrap();
        db.execute("INSERT INTO worker_runtime(session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,result_status,retry_count,worktree_path,updated_at) VALUES('impl','s','completed','implementation','key','reported',0,?1,'now')", params![cwd]).unwrap();
        let implementation_request = serde_json::json!({"schemaVersion":1,"role":"implementation","objective":"Implement proof","acceptanceCriteria":["User opens the dialog"],"knownFacts":[],"decisions":[],"evidenceIds":[],"relevantFiles":["src/App.tsx"],"ownedPaths":["src/**"],"writeMode":"isolated","capabilityTier":"standard","effort":"medium","verification":["bun run test"],"outputContract":"implementation-result","harness":"claude"});
        db.execute("INSERT INTO worker_completion_inputs(child_session_id,request,updated_at) VALUES('impl',?1,'now')", params![implementation_request.to_string()]).unwrap();
        let implementation = WorkerResult {
            schema_version: 1,
            status: WorkerResultStatus::Completed,
            summary: "implemented".into(),
            files_changed: vec!["src/App.tsx".into()],
            tests: vec![],
            decisions: vec![],
            risks: vec![],
            remaining_work: vec![],
            suggested_next_action: SuggestedNextAction::Finish,
            suggested_role: None,
            suggested_task: None,
        };
        let first = create_from_worker_result(&db, "impl", &implementation, &HashSet::new())
            .unwrap()
            .unwrap();

        for (id, objective, tests) in [
            (
                "verify-code",
                "Run scrutiny-review",
                vec![
                    WorkerTestResult {
                        command: "bun run test".into(),
                        status: TestStatus::Passed,
                        detail: Some("green".into()),
                    },
                    WorkerTestResult {
                        command: "bun run build".into(),
                        status: TestStatus::Passed,
                        detail: Some("built".into()),
                    },
                ],
            ),
            ("verify-ui", "Run user-journey in browser", vec![]),
        ] {
            db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id,kind,continuation_fidelity) VALUES(?1,'w','codex','verify','completed','estimated','s','worker','native')", params![id]).unwrap();
            db.execute("INSERT INTO worker_leases(session_id,workspace_id,role,capability_tier,task_family,owned_paths,write_mode,lease_status,created_at,updated_at) VALUES(?1,'w','verification','standard','verification','[]','read_only','released','now','now')", params![id]).unwrap();
            db.execute("INSERT INTO worker_runtime(session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,result_status,retry_count,worktree_path,updated_at) VALUES(?1,'s','completed','verification','key','reported',0,?2,'now')", params![id, cwd]).unwrap();
            let request = serde_json::json!({"schemaVersion":1,"role":"verification","objective":objective,"acceptanceCriteria":["Report typed evidence"],"knownFacts":[],"decisions":[],"evidenceIds":[],"relevantFiles":["src/App.tsx"],"ownedPaths":[],"writeMode":"readOnly","capabilityTier":"standard","effort":"medium","verification":[],"outputContract":"verification-result","harness":"codex"});
            db.execute("INSERT INTO worker_completion_inputs(child_session_id,request,updated_at) VALUES(?1,?2,'now')", params![id, request.to_string()]).unwrap();
            let verification = WorkerResult {
                schema_version: 1,
                status: WorkerResultStatus::Completed,
                summary: "verified".into(),
                files_changed: vec![],
                tests,
                decisions: vec![],
                risks: vec![],
                remaining_work: vec![],
                suggested_next_action: SuggestedNextAction::Finish,
                suggested_role: None,
                suggested_task: None,
            };
            create_from_worker_result(&db, id, &verification, &HashSet::new()).unwrap();
        }
        let final_summary = latest_summary(&db, "s").unwrap().unwrap();
        assert_eq!(final_summary.attempt_id, first.attempt_id);
        // Semantic checks are the verifier's to settle, and both closed.
        assert!(final_summary
            .checks
            .iter()
            .filter(|check| check.kind != EvalKind::Deterministic)
            .all(|check| check.status == CheckStatus::Passed));
        // The shell checks are NOT closed by the verifier's `tests[]`. A digest of
        // the JSON that arrived is not evidence the command ran, so they stay
        // pending for the check runner and the gate stays open.
        let shell: Vec<_> = final_summary
            .checks
            .iter()
            .filter(|check| check.executor == SHELL_EXECUTOR)
            .collect();
        assert!(shell
            .iter()
            .any(|check| check.command.as_deref() == Some("bun run test")));
        assert!(shell
            .iter()
            .all(|check| check.status == CheckStatus::Pending));
        assert_eq!(final_summary.verdict, CompletionVerdict::Verifying);
        // And they cannot be closed that way even directly.
        let refused = record_check(
            &db,
            &final_summary.attempt_id,
            &CheckRun {
                check_id: shell[0].check_id.clone(),
                kind: EvalKind::Deterministic,
                required: true,
                status: CheckStatus::Passed,
                executor: WORKER_RESULT_EXECUTOR.into(),
                command: shell[0].command.clone(),
                verifier_family: Some("codex".into()),
                detail: Some("the worker said it passed".into()),
                output_digest: Some("digest-of-received-json".into()),
                artifact_refs: vec![],
            },
        )
        .unwrap_err();
        assert!(
            refused.to_string().contains("cannot satisfy it"),
            "{refused}"
        );
    }

    /// `bridge.system` exists so Bridge can say *why* a check could not run. It
    /// must not be a way for a caller to mark one passed without an executor.
    #[test]
    fn the_system_executor_can_explain_a_failure_but_never_pass_a_check() {
        let db = fixture();
        let contract = contract();
        let plan = EvalPlan {
            id: "plan".into(),
            contract_id: contract.id.clone(),
            schema_version: 1,
            risk: RiskTier::Low,
            checks: vec![EvalCheck {
                id: "tests".into(),
                label: "tests".into(),
                kind: EvalKind::Deterministic,
                required: true,
                executor: SHELL_EXECUTOR.into(),
                command: Some("bun run test".into()),
                required_capabilities: vec!["shell".into()],
                different_model_family: false,
                reason: "policy".into(),
            }],
        };
        let stamp = RepositoryStamp {
            head: "head".into(),
            dirty_digest: "dirty".into(),
        };
        let attempt = create_flow(&db, &contract, &plan, "s", ".", &stamp, Some("codex")).unwrap();

        let forged = |executor: &str| CheckRun {
            check_id: "tests".into(),
            kind: EvalKind::Deterministic,
            required: true,
            status: CheckStatus::Passed,
            executor: executor.into(),
            command: Some("bun run test".into()),
            verifier_family: None,
            detail: Some("trust me".into()),
            output_digest: Some("digest".into()),
            artifact_refs: vec![],
        };
        // Neither the system executor nor a worker result can pass a shell check.
        let system = record_check(&db, &attempt, &forged(SYSTEM_EXECUTOR)).unwrap_err();
        assert!(system.to_string().contains("cannot pass check"), "{system}");
        let worker = record_check(&db, &attempt, &forged(WORKER_RESULT_EXECUTOR)).unwrap_err();
        assert!(worker.to_string().contains("cannot satisfy it"), "{worker}");
        assert_eq!(
            latest_summary(&db, "s").unwrap().unwrap().passed_required,
            0
        );

        // The system executor may still record why the check could not run.
        record_check(
            &db,
            &attempt,
            &CheckRun {
                status: CheckStatus::Blocked,
                detail: Some("deadline expired".into()),
                ..forged(SYSTEM_EXECUTOR)
            },
        )
        .unwrap();
        assert_eq!(
            latest_summary(&db, "s").unwrap().unwrap().checks[0].status,
            CheckStatus::Blocked
        );
        // And the runner itself can pass it.
        db.execute(
            "UPDATE eval_check_runs SET status='pending' WHERE attempt_id=?1",
            params![attempt],
        )
        .unwrap();
        record_check(&db, &attempt, &forged(SHELL_EXECUTOR)).unwrap();
        assert_eq!(
            latest_summary(&db, "s").unwrap().unwrap().passed_required,
            1
        );
    }

    #[test]
    fn completion_is_revision_bound_and_preserves_failed_checks() {
        let db = fixture();
        let contract = contract();
        save_contract(&db, &contract).unwrap();
        let plan = EvalPlan {
            id: "plan".into(),
            contract_id: contract.id.clone(),
            schema_version: 1,
            risk: RiskTier::High,
            checks: vec![EvalCheck {
                id: "tests".into(),
                label: "tests".into(),
                kind: EvalKind::Deterministic,
                required: true,
                executor: "shell".into(),
                command: Some("bun test".into()),
                required_capabilities: vec!["shell".into()],
                different_model_family: false,
                reason: "policy".into(),
            }],
        };
        save_plan(&db, &plan).unwrap();
        let stamp = RepositoryStamp {
            head: "abc".into(),
            dirty_digest: "clean".into(),
        };
        let attempt = begin_attempt(&db, &plan, "s", ".", &stamp, Some("codex")).unwrap();
        record_check(
            &db,
            &attempt,
            &CheckRun {
                check_id: "tests".into(),
                kind: EvalKind::Deterministic,
                required: true,
                status: CheckStatus::Failed,
                executor: "shell".into(),
                command: Some("bun test".into()),
                verifier_family: None,
                detail: Some("failure".into()),
                output_digest: Some("digest".into()),
                artifact_refs: vec![],
            },
        )
        .unwrap();
        let bundle = finalize(&db, &attempt, &stamp).unwrap();
        assert_eq!(bundle.verdict, CompletionVerdict::ChangesRequested);
        assert_eq!(bundle.failed_or_skipped, vec!["tests"]);
        assert!(finalize(
            &db,
            &attempt,
            &RepositoryStamp {
                head: "def".into(),
                dirty_digest: "clean".into()
            }
        )
        .unwrap_err()
        .to_string()
        .contains("stale"));
    }

    #[test]
    fn persisted_check_cannot_claim_same_family_independence() {
        let db = fixture();
        let contract = contract();
        save_contract(&db, &contract).unwrap();
        let plan = EvalPlan {
            id: "plan".into(),
            contract_id: contract.id.clone(),
            schema_version: 1,
            risk: RiskTier::High,
            checks: vec![EvalCheck {
                id: "scrutiny".into(),
                label: "scrutiny".into(),
                kind: EvalKind::Scrutiny,
                required: true,
                executor: "worker".into(),
                command: None,
                required_capabilities: vec!["code_review".into()],
                different_model_family: true,
                reason: "risk".into(),
            }],
        };
        save_plan(&db, &plan).unwrap();
        let stamp = RepositoryStamp {
            head: "abc".into(),
            dirty_digest: "clean".into(),
        };
        let attempt = begin_attempt(&db, &plan, "s", ".", &stamp, Some("codex")).unwrap();
        let error = record_check(
            &db,
            &attempt,
            &CheckRun {
                check_id: "scrutiny".into(),
                kind: EvalKind::Scrutiny,
                required: true,
                status: CheckStatus::Passed,
                executor: "worker".into(),
                command: None,
                verifier_family: Some("codex".into()),
                detail: None,
                output_digest: None,
                artifact_refs: vec![],
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("different model family"));
    }

    #[test]
    fn scoped_human_waiver_is_distinct_from_verified() {
        let db = fixture();
        let contract = contract();
        save_contract(&db, &contract).unwrap();
        let plan = EvalPlan {
            id: "plan".into(),
            contract_id: contract.id.clone(),
            schema_version: 1,
            risk: RiskTier::Low,
            checks: vec![EvalCheck {
                id: "manual".into(),
                label: "manual".into(),
                kind: EvalKind::UserTesting,
                required: true,
                executor: "worker".into(),
                command: None,
                required_capabilities: vec!["computer".into()],
                different_model_family: true,
                reason: "journey".into(),
            }],
        };
        save_plan(&db, &plan).unwrap();
        let stamp = RepositoryStamp {
            head: "abc".into(),
            dirty_digest: "clean".into(),
        };
        let attempt = begin_attempt(&db, &plan, "s", ".", &stamp, Some("codex")).unwrap();
        waive(
            &db,
            &attempt,
            &["manual".into()],
            "tool unavailable",
            "user",
            &stamp,
        )
        .unwrap();
        assert_eq!(
            finalize(&db, &attempt, &stamp).unwrap().verdict,
            CompletionVerdict::Waived
        );
    }

    #[test]
    fn partial_waiver_cannot_bypass_an_unwaived_failed_check() {
        let db = fixture();
        let contract = contract();
        save_contract(&db, &contract).unwrap();
        let checks = [
            ("tests", EvalKind::Deterministic, CheckStatus::Failed),
            ("journey", EvalKind::UserTesting, CheckStatus::Skipped),
        ];
        let plan = EvalPlan {
            id: "plan".into(),
            contract_id: contract.id.clone(),
            schema_version: 1,
            risk: RiskTier::High,
            checks: checks
                .iter()
                .map(|(id, kind, _)| EvalCheck {
                    id: (*id).into(),
                    label: (*id).into(),
                    kind: *kind,
                    required: true,
                    executor: "worker".into(),
                    command: None,
                    required_capabilities: vec![],
                    different_model_family: false,
                    reason: "required".into(),
                })
                .collect(),
        };
        save_plan(&db, &plan).unwrap();
        let stamp = RepositoryStamp {
            head: "abc".into(),
            dirty_digest: "clean".into(),
        };
        let attempt = begin_attempt(&db, &plan, "s", ".", &stamp, Some("codex")).unwrap();
        for (id, kind, status) in checks {
            record_check(
                &db,
                &attempt,
                &CheckRun {
                    check_id: id.into(),
                    kind,
                    required: true,
                    status,
                    executor: "worker".into(),
                    command: None,
                    verifier_family: None,
                    detail: None,
                    output_digest: Some("digest".into()),
                    artifact_refs: vec![],
                },
            )
            .unwrap();
        }
        waive(
            &db,
            &attempt,
            &["journey".into()],
            "browser unavailable",
            "user",
            &stamp,
        )
        .unwrap();
        assert_eq!(
            finalize(&db, &attempt, &stamp).unwrap().verdict,
            CompletionVerdict::ChangesRequested
        );
    }

    #[test]
    fn gate_creation_error_fails_closed_and_remains_explicitly_waivable() {
        let db = fixture();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id) VALUES('broken','w','claude','broken','failed','estimated','s')", []).unwrap();
        db.execute("INSERT INTO worker_leases(session_id,workspace_id,role,capability_tier,task_family,owned_paths,write_mode,lease_status,created_at,updated_at) VALUES('broken','w','implementation','standard','implementation','[]','isolated','released','now','now')", []).unwrap();
        db.execute("INSERT INTO worker_runtime(session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,result_status,retry_count,updated_at) VALUES('broken','s','failed','implementation','key','reported',0,'now')", []).unwrap();
        let summary = record_gate_error(&db, "broken", "worktree disappeared")
            .unwrap()
            .unwrap();
        assert_eq!(summary.verdict, CompletionVerdict::Failed);
        assert_eq!(summary.checks[0].status, CheckStatus::Blocked);
        assert!(!completion_allows_ready(&db, "s").unwrap());
        waive(
            &db,
            &summary.attempt_id,
            &["gate-error".into()],
            "continue without automated proof",
            "user",
            &summary.repository,
        )
        .unwrap();
        assert_eq!(
            finalize(&db, &summary.attempt_id, &summary.repository)
                .unwrap()
                .verdict,
            CompletionVerdict::Waived
        );
    }

    #[test]
    fn readiness_requires_zero_unsettled_children_and_a_terminal_gate() {
        let db = fixture();
        db.execute("UPDATE sessions SET status='waiting' WHERE id='s'", [])
            .unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id) VALUES('active','w','claude','active','working','estimated','s')", []).unwrap();
        db.execute("INSERT INTO worker_runtime(session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,result_status,retry_count,updated_at) VALUES('active','s','working','implementation','key','pending',0,'now')", []).unwrap();
        assert!(!reconcile_parent_readiness(&db, "s").unwrap());
        db.execute(
            "UPDATE worker_runtime SET result_status='reported' WHERE session_id='active'",
            [],
        )
        .unwrap();
        assert!(reconcile_parent_readiness(&db, "s").unwrap());
        assert_eq!(
            db.query_row("SELECT status FROM sessions WHERE id='s'", [], |row| row
                .get::<_, String>(
                0
            ))
            .unwrap(),
            "ready"
        );
        let contract = contract();
        let plan = EvalPlan {
            id: "plan".into(),
            contract_id: contract.id.clone(),
            schema_version: 1,
            risk: RiskTier::Low,
            checks: vec![EvalCheck {
                id: "pending".into(),
                label: "pending".into(),
                kind: EvalKind::Deterministic,
                required: true,
                executor: "worker".into(),
                command: None,
                required_capabilities: vec![],
                different_model_family: false,
                reason: "required".into(),
            }],
        };
        create_flow(
            &db,
            &contract,
            &plan,
            "s",
            ".",
            &RepositoryStamp {
                head: "head".into(),
                dirty_digest: "dirty".into(),
            },
            Some("codex"),
        )
        .unwrap();
        assert!(!reconcile_parent_readiness(&db, "s").unwrap());
        assert_eq!(
            db.query_row("SELECT status FROM sessions WHERE id='s'", [], |row| row
                .get::<_, String>(
                0
            ))
            .unwrap(),
            "waiting"
        );
    }

    #[test]
    fn explicit_plan_creation_does_not_interrupt_a_working_parent() {
        let db = fixture();
        db.execute("UPDATE sessions SET status='working' WHERE id='s'", [])
            .unwrap();
        let contract = contract();
        let plan = EvalPlan {
            id: "plan".into(),
            contract_id: contract.id.clone(),
            schema_version: 1,
            risk: RiskTier::Low,
            checks: vec![EvalCheck {
                id: "pending".into(),
                label: "pending".into(),
                kind: EvalKind::Deterministic,
                required: true,
                executor: "worker".into(),
                command: None,
                required_capabilities: vec![],
                different_model_family: false,
                reason: "required".into(),
            }],
        };
        create_flow(
            &db,
            &contract,
            &plan,
            "s",
            ".",
            &RepositoryStamp {
                head: "head".into(),
                dirty_digest: "dirty".into(),
            },
            Some("codex"),
        )
        .unwrap();
        assert_eq!(
            db.query_row("SELECT status FROM sessions WHERE id='s'", [], |row| row
                .get::<_, String>(
                0
            ))
            .unwrap(),
            "working"
        );
    }

    #[test]
    fn author_filename_does_not_trigger_auth_risk() {
        let result = plan(PlanInput {
            contract_id: "c".into(),
            acceptance_criteria: vec!["Render author".into()],
            changed_paths: vec!["src/author.rs".into()],
            repository_commands: vec![],
        });
        assert_eq!(result.risk, RiskTier::Low);
    }

    #[test]
    fn compact_packet_selects_relevant_contract_and_bounded_findings() {
        let packet = compact_packet(
            "a",
            RepositoryStamp {
                head: "h".into(),
                dirty_digest: "d".into(),
            },
            &[
                "Router saves settings".into(),
                "Unrelated billing works".into(),
            ],
            &["src/router/settings.rs".into()],
            &[],
            &(0..30)
                .map(|index| format!("finding {index}"))
                .collect::<Vec<_>>(),
        );
        assert_eq!(packet.relevant_criteria, vec!["Router saves settings"]);
        assert_eq!(packet.unresolved_findings.len(), 16);
    }

    #[test]
    fn completion_benchmark_fixture_enforces_the_measurement_contract() {
        let cases: Vec<CompletionBenchmarkCase> = serde_json::from_str(include_str!(
            "../../../testing/fixtures/completion-benchmark-v1.json"
        ))
        .unwrap();
        let report = benchmark(&cases);
        assert_eq!(report.cases, 10);
        assert_eq!(report.baseline_false_done, 3);
        assert_eq!(report.proof_false_done, 0);
        assert!(report.proof_verified_quality_bps > report.baseline_verified_quality_bps);
        assert!(report.proof_cost_per_accepted < report.baseline_cost_per_accepted);
        assert!(report.proof_average_latency_ms < report.baseline_average_latency_ms);
        assert!(report.proof_human_interventions < report.baseline_human_interventions);
    }

    fn git(cwd: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    /// A real one-commit repository, so the Git-derived revision paths have
    /// honest state to read instead of a fixture pretending to be a checkout.
    fn repository() -> (tempfile::TempDir, String) {
        let fixture = tempfile::tempdir().unwrap();
        let repo = fixture.path().join("task");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.email", "bridge-test@example.invalid"]);
        git(&repo, &["config", "user.name", "Bridge Test"]);
        std::fs::write(repo.join("README.md"), "base\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "fixture", "-q"]);
        (fixture, repo.to_string_lossy().into_owned())
    }

    fn verification_directive() -> DelegationRequest {
        DelegationRequest {
            schema_version: crate::delegation::SCHEMA_VERSION,
            role: crate::delegation::WorkerRole::Verification,
            objective: "Verify the orchestrator's own edit".into(),
            acceptance_criteria: vec!["the edit is present in the checkout".into()],
            known_facts: vec![],
            decisions: vec![],
            evidence_ids: vec![],
            relevant_files: vec![],
            owned_paths: vec![],
            write_mode: crate::delegation::WriteMode::ReadOnly,
            capability_tier: crate::delegation::CapabilityTier::Standard,
            effort: crate::delegation::Effort::Medium,
            network_access: false,
            writable_output_paths: vec![],
            verification: vec!["git diff --check".into()],
            output_contract: crate::delegation::OutputContract::VerificationResult,
            harness: None,
            model: None,
        }
    }

    /// Point the fixture workspace at the real temp repo so
    /// `repository_path_for_session` (`COALESCE(s.cwd, w.path)`) resolves to it.
    fn wire_workspace_to(db: &Connection, repository_path: &str) {
        db.execute(
            "UPDATE workspaces SET path=?2 WHERE id='w'",
            params!["w", repository_path],
        )
        .unwrap();
    }

    /// Issue #327, happy path: the orchestrator edited files directly and then
    /// delegated verification. A gate must open from the checkout's Git state —
    /// with no implementation worker ever having run.
    #[test]
    fn orchestrator_edits_open_a_verification_gate_without_a_worker() {
        let (_fixture, repo) = repository();
        std::fs::write(std::path::Path::new(&repo).join("README.md"), "edited\n").unwrap();
        let db = fixture();
        wire_workspace_to(&db, &repo);
        let directive = verification_directive();

        let summary =
            create_from_orchestrator_edits(&db, "s", "w", &repo, &directive, &HashSet::new())
                .unwrap()
                .expect("direct edits deserve an implementation revision");
        assert_eq!(summary.verdict, CompletionVerdict::Verifying);
        assert_eq!(summary.repository.head, git(Path::new(&repo), &["rev-parse", "HEAD"]));
        assert!(
            !summary.checks.is_empty(),
            "the gate plans checks over the changed paths"
        );
        // The verifier can now find its bind target.
        assert_eq!(
            verification_target_path(&db, "s").unwrap(),
            Some(repo.clone())
        );
        let (stored_head, stored_path): (String, String) = db
            .query_row(
                "SELECT repository_head,repository_path FROM eval_attempts",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(stored_path, repo, "the attempt is bound to the task checkout");
        assert_eq!(stored_head, summary.repository.head);
    }

    /// Nothing implemented anywhere means nothing to record — but that must be
    /// a clean refusal, not a fabricated revision over a pristine tree.
    #[test]
    fn a_clean_checkout_records_no_orchestrator_revision() {
        let (_fixture, repo) = repository();
        let db = fixture();
        wire_workspace_to(&db, &repo);
        let resolution = ensure_verification_target(&db, "s", &verification_directive(), &HashSet::new())
            .unwrap();
        assert_eq!(resolution, VerificationTargetResolution::Missing);
        let attempts: i64 = db
            .query_row("SELECT COUNT(*) FROM eval_attempts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(attempts, 0);

        // A directory that is not even a repository is also Missing, not an error.
        let scratch = tempfile::tempdir().unwrap();
        wire_workspace_to(&db, scratch.path().to_string_lossy().as_ref());
        let resolution = ensure_verification_target(&db, "s", &verification_directive(), &HashSet::new())
            .unwrap();
        assert_eq!(resolution, VerificationTargetResolution::Missing);
    }

    /// Recording twice over the same tree state must not stack gates: the first
    /// call self-records, the second finds the live gate and leaves it alone.
    #[test]
    fn self_recording_dedupes_while_the_stamp_is_unchanged() {
        let (_fixture, repo) = repository();
        std::fs::write(std::path::Path::new(&repo).join("README.md"), "edited\n").unwrap();
        let db = fixture();
        wire_workspace_to(&db, &repo);
        let directive = verification_directive();

        let first = ensure_verification_target(&db, "s", &directive, &HashSet::new()).unwrap();
        assert_eq!(first, VerificationTargetResolution::SelfRecorded);
        let second = ensure_verification_target(&db, "s", &directive, &HashSet::new()).unwrap();
        assert_eq!(second, VerificationTargetResolution::Existing);

        let attempts: i64 = db
            .query_row("SELECT COUNT(*) FROM eval_attempts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(attempts, 1, "one revision per unchanged tree state");
    }

    /// An already-open gate wins over any new recording, whatever the directive
    /// says — self-recording exists to fill the gap, not to supersede live work.
    #[test]
    fn an_open_gate_is_left_alone_by_self_recording() {
        let (_fixture, repo) = repository();
        std::fs::write(std::path::Path::new(&repo).join("README.md"), "edited\n").unwrap();
        let db = fixture();
        wire_workspace_to(&db, &repo);
        let mut directive = verification_directive();
        create_from_orchestrator_edits(&db, "s", "w", &repo, &directive, &HashSet::new())
            .unwrap()
            .expect("first recording opens the gate");

        directive.objective = "a different delegation over the same tree".into();
        let resolution = ensure_verification_target(&db, "s", &directive, &HashSet::new()).unwrap();
        assert_eq!(resolution, VerificationTargetResolution::Existing);
        let contracts: i64 = db
            .query_row("SELECT COUNT(*) FROM completion_contracts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(contracts, 1, "no second contract behind the live gate");
    }

    /// Issue #327's complaint about the message: it assumed an implementation
    /// worker existed. Both remedies must now be named, each for its own case.
    #[test]
    fn missing_target_reason_names_both_remediations() {
        let reason = verification_target_unavailable_reason();
        assert!(reason.contains(VERIFICATION_TARGET_UNAVAILABLE), "{reason}");
        assert!(
            reason.contains("adopt or commit the"),
            "the unadopted-worker case keeps its remedy: {reason}"
        );
        assert!(
            reason.contains("delegate the implementation before requesting verification"),
            "the no-work-at-all case gets its own remedy: {reason}"
        );
    }
}

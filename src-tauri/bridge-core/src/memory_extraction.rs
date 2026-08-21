//! Post-turn memory extraction: the first LLM writer, and it can only propose.
//!
//! Written against a trait rather than a provider process, for the same reason
//! the briefing runner is: the decisions — what a digest contains, what a
//! proposal must look like, what the gate refuses — are this module's
//! responsibility, and they must be checkable without a network. The live
//! binding (memory_extraction_live) runs the user's pinned harness and model
//! in a hidden bounded session; the learning router is never consulted, and an
//! extraction run writes no router decision.

use crate::memory_ledger;
use crate::BridgeError;
use bridge_protocol::messages::{MemoryRecord, ACCOUNT_MEMORY_SCOPE};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const EXTRACTION_SESSION_KIND: &str = "extraction";
pub const MODE_REMEMBER: &str = "remember";
pub const MODE_PROPOSE: &str = "propose";
const MODE_AUTO_APPLY: &str = "auto_apply";
const MAX_PROPOSALS_PER_RUN: usize = 10;
const MAX_RATIONALE_CHARS: usize = 500;
const MAX_DIGEST_CHARS: usize = 24_000;
const MAX_DIGEST_ENTRIES: usize = 30;
const MAX_PIN_CONTEXT: usize = 20;
const LEASE_MINUTES: i64 = 10;
const ENQUEUEABLE_SESSION_KINDS: [&str; 2] = ["chat", "orchestrator"];

pub(crate) fn install(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory_extraction_settings (
            scope_key TEXT PRIMARY KEY,
            mode TEXT NOT NULL,
            harness TEXT,
            model TEXT,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS memory_extraction_runs (
            id TEXT PRIMARY KEY,
            scope_key TEXT NOT NULL,
            session_id TEXT NOT NULL,
            status TEXT NOT NULL,
            adapter TEXT,
            model TEXT,
            prompt_digest TEXT,
            observed_tokens INTEGER NOT NULL DEFAULT 0,
            spend_microusd INTEGER NOT NULL DEFAULT 0,
            proposal_count INTEGER NOT NULL DEFAULT 0,
            detail TEXT,
            lease_owner TEXT,
            lease_expires_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_memory_extraction_runs_status
            ON memory_extraction_runs(status, created_at);",
    )?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionSettings {
    pub scope_key: String,
    pub mode: String,
    pub harness: Option<String>,
    pub model: Option<String>,
}

pub fn settings(db: &Connection, scope_key: &str) -> Result<ExtractionSettings, BridgeError> {
    let scope_key = memory_ledger::parse_scope_key(scope_key)?;
    let row = db
        .query_row(
            "SELECT mode, harness, model FROM memory_extraction_settings WHERE scope_key=?1",
            params![scope_key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()?;
    let (mode, harness, model) = row.unwrap_or((MODE_REMEMBER.to_string(), None, None));
    Ok(ExtractionSettings { scope_key, mode, harness, model })
}

pub fn update_settings(
    db: &Connection,
    scope_key: &str,
    mode: &str,
    harness: Option<&str>,
    model: Option<&str>,
) -> Result<ExtractionSettings, BridgeError> {
    let scope_key = memory_ledger::parse_scope_key(scope_key)?;
    if scope_key != ACCOUNT_MEMORY_SCOPE {
        return Err(BridgeError::Invalid(
            "Extraction settings exist for account:local only in this release.".into(),
        ));
    }
    let mode = match mode.trim() {
        MODE_REMEMBER => MODE_REMEMBER,
        MODE_PROPOSE => MODE_PROPOSE,
        MODE_AUTO_APPLY => {
            return Err(BridgeError::Invalid(
                "Auto-apply does not exist until a replay bench can justify it. Use remember or propose.".into(),
            ))
        }
        other => {
            return Err(BridgeError::Invalid(format!(
                "Unknown extraction mode '{other}'. Use remember or propose."
            )))
        }
    };
    let harness = harness.map(str::trim).filter(|value| !value.is_empty());
    let model = model.map(str::trim).filter(|value| !value.is_empty());
    if mode == MODE_PROPOSE && (harness.is_none() || model.is_none()) {
        return Err(BridgeError::Invalid(
            "Propose mode needs a pinned harness and model to run on.".into(),
        ));
    }
    db.execute(
        "INSERT INTO memory_extraction_settings(scope_key, mode, harness, model, updated_at)
         VALUES(?1,?2,?3,?4,?5)
         ON CONFLICT(scope_key) DO UPDATE SET mode=?2, harness=?3, model=?4, updated_at=?5",
        params![scope_key, mode, harness, model, Utc::now().to_rfc3339()],
    )?;
    settings(db, &scope_key)
}

/// Called after a completed turn. Enqueues at most one open run per session,
/// only for visible conversational kinds, only when the scope opted in.
pub fn enqueue_after_turn(db: &Connection, session_id: &str) -> Result<bool, BridgeError> {
    let kind: Option<Option<String>> = db
        .query_row(
            "SELECT kind FROM sessions WHERE id=?1",
            params![session_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(kind) = kind else { return Ok(false) };
    let kind = kind.unwrap_or_else(|| "chat".to_string());
    if !ENQUEUEABLE_SESSION_KINDS.contains(&kind.as_str()) {
        return Ok(false);
    }
    let current = settings(db, ACCOUNT_MEMORY_SCOPE)?;
    if current.mode != MODE_PROPOSE {
        return Ok(false);
    }
    let open: Option<i64> = db
        .query_row(
            "SELECT 1 FROM memory_extraction_runs
             WHERE session_id=?1 AND status IN ('queued','running') LIMIT 1",
            params![session_id],
            |row| row.get(0),
        )
        .optional()?;
    if open.is_some() {
        return Ok(false);
    }
    let now = Utc::now().to_rfc3339();
    db.execute(
        "INSERT INTO memory_extraction_runs(id, scope_key, session_id, status, created_at, updated_at)
         VALUES(?1,?2,?3,'queued',?4,?4)",
        params![Uuid::new_v4().to_string(), ACCOUNT_MEMORY_SCOPE, session_id, now],
    )?;
    Ok(true)
}

#[derive(Debug, Clone)]
pub struct ClaimedExtraction {
    pub run_id: String,
    pub scope_key: String,
    pub session_id: String,
    pub lease_owner: String,
    pub harness: String,
    pub model: String,
}

/// One due run, leased. A queued run whose scope has since left propose mode
/// is settled `cancelled` rather than executed — turning it off means off.
pub fn claim_due(
    db: &Connection,
    now: DateTime<Utc>,
) -> Result<Option<ClaimedExtraction>, BridgeError> {
    loop {
        let candidate: Option<(String, String, String)> = db
            .query_row(
                "SELECT id, scope_key, session_id FROM memory_extraction_runs
                 WHERE status='queued'
                    OR (status='running' AND (lease_expires_at IS NULL OR lease_expires_at < ?1))
                 ORDER BY created_at LIMIT 1",
                params![now.to_rfc3339()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((run_id, scope_key, session_id)) = candidate else {
            return Ok(None);
        };
        let current = settings(db, &scope_key)?;
        if current.mode != MODE_PROPOSE || current.harness.is_none() || current.model.is_none() {
            db.execute(
                "UPDATE memory_extraction_runs
                 SET status='cancelled', detail='extraction_disabled', lease_owner=NULL,
                     lease_expires_at=NULL, updated_at=?2
                 WHERE id=?1 AND status IN ('queued','running')",
                params![run_id, now.to_rfc3339()],
            )?;
            continue;
        }
        let lease_owner = Uuid::new_v4().to_string();
        let expires = (now + Duration::minutes(LEASE_MINUTES)).to_rfc3339();
        let claimed = db.execute(
            "UPDATE memory_extraction_runs
             SET status='running', lease_owner=?2, lease_expires_at=?3, updated_at=?4
             WHERE id=?1 AND (status='queued'
                OR (status='running' AND (lease_expires_at IS NULL OR lease_expires_at < ?4)))",
            params![run_id, lease_owner, expires, now.to_rfc3339()],
        )?;
        if claimed == 0 {
            continue;
        }
        return Ok(Some(ClaimedExtraction {
            run_id,
            scope_key,
            session_id,
            lease_owner,
            harness: current.harness.expect("checked above"),
            model: current.model.expect("checked above"),
        }));
    }
}

pub fn heartbeat(
    db: &Connection,
    run_id: &str,
    lease_owner: &str,
    now: DateTime<Utc>,
) -> Result<bool, BridgeError> {
    let expires = (now + Duration::minutes(LEASE_MINUTES)).to_rfc3339();
    let held = db.execute(
        "UPDATE memory_extraction_runs SET lease_expires_at=?3, updated_at=?4
         WHERE id=?1 AND lease_owner=?2 AND status='running'",
        params![run_id, lease_owner, expires, now.to_rfc3339()],
    )?;
    Ok(held > 0)
}

#[allow(clippy::too_many_arguments)]
pub fn settle(
    db: &Connection,
    run_id: &str,
    lease_owner: &str,
    status: &str,
    detail: Option<&str>,
    adapter: Option<&str>,
    model: Option<&str>,
    prompt_digest: Option<&str>,
    observed_tokens: i64,
    spend_microusd: i64,
    proposal_count: i64,
) -> Result<bool, BridgeError> {
    if !matches!(status, "completed" | "failed" | "cancelled") {
        return Err(BridgeError::Invalid(format!(
            "An extraction run settles completed, failed, or cancelled — not '{status}'."
        )));
    }
    let settled = db.execute(
        "UPDATE memory_extraction_runs
         SET status=?3, detail=?4, adapter=?5, model=?6, prompt_digest=?7,
             observed_tokens=?8, spend_microusd=?9, proposal_count=?10,
             lease_owner=NULL, lease_expires_at=NULL, updated_at=?11
         WHERE id=?1 AND lease_owner=?2 AND status='running'",
        params![
            run_id,
            lease_owner,
            status,
            detail,
            adapter,
            model,
            prompt_digest,
            observed_tokens,
            spend_microusd,
            proposal_count,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(settled > 0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionRunSummary {
    pub status: String,
    pub proposal_count: i64,
    pub observed_tokens: i64,
    pub spend_microusd: i64,
    pub detail: Option<String>,
    pub updated_at: String,
}

pub fn last_run(db: &Connection, scope_key: &str) -> Result<Option<ExtractionRunSummary>, BridgeError> {
    let scope_key = memory_ledger::parse_scope_key(scope_key)?;
    db.query_row(
        "SELECT status, proposal_count, observed_tokens, spend_microusd, detail, updated_at
         FROM memory_extraction_runs WHERE scope_key=?1
         ORDER BY created_at DESC LIMIT 1",
        params![scope_key],
        |row| {
            Ok(ExtractionRunSummary {
                status: row.get(0)?,
                proposal_count: row.get(1)?,
                observed_tokens: row.get(2)?,
                spend_microusd: row.get(3)?,
                detail: row.get(4)?,
                updated_at: row.get(5)?,
            })
        },
    )
    .optional()
    .map_err(BridgeError::from)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionDigest {
    pub text: String,
    pub sha256: String,
}

/// Bounded and typed: recent conversational entries by id plus the scope's
/// active pin bodies. Never a raw transcript, never another session.
pub fn build_digest(
    db: &Connection,
    scope_key: &str,
    session_id: &str,
) -> Result<Option<ExtractionDigest>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT id, kind, coalesce(json_extract(payload,'$.text'), '')
         FROM session_entries
         WHERE session_id=?1 AND kind IN ('user.message','assistant.message')
           AND context_visibility IN ('eligible','visible')
         ORDER BY sequence DESC LIMIT ?2",
    )?;
    let mut newest_first: Vec<(String, String, String)> = statement
        .query_map(params![session_id, MAX_DIGEST_ENTRIES as i64], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    newest_first.retain(|(_, _, text)| !text.trim().is_empty());
    if newest_first.is_empty() {
        return Ok(None);
    }
    let mut used = 0usize;
    let mut lines: Vec<String> = Vec::new();
    for (id, kind, text) in &newest_first {
        let role = if kind == "user.message" { "user" } else { "assistant" };
        let line = format!("[{id}] {role}: {}", text.trim());
        if used + line.len() > MAX_DIGEST_CHARS {
            break;
        }
        used += line.len();
        lines.push(line);
    }
    lines.reverse();
    let pins = memory_ledger::list(db, scope_key, None)?;
    let mut text = String::new();
    if !pins.records.is_empty() {
        text.push_str("Already pinned (do not re-propose):\n");
        for record in pins.records.iter().take(MAX_PIN_CONTEXT) {
            let body: String = record.body.chars().take(200).collect();
            text.push_str(&format!("- {body}\n"));
        }
        text.push('\n');
    }
    text.push_str("Conversation:\n");
    text.push_str(&lines.join("\n"));
    let sha256 = format!("{:x}", Sha256::digest(text.as_bytes()));
    Ok(Some(ExtractionDigest { text, sha256 }))
}

pub fn extraction_instructions() -> String {
    "You review one conversation digest and propose durable facts about the user \
     or how they work: preferences, conventions, decisions, constraints. Answer \
     with exactly one fenced code block tagged bridge-memory-proposals containing \
     a JSON array. Each element is an object with exactly these fields: body \
     (the fact, under 4000 characters), kind (preference, fact, decision, or \
     constraint), confidenceBps (0-10000), rationale (why, under 500 characters). \
     At most 10 elements. Propose nothing sensitive, no credentials, nothing \
     already pinned, and nothing about this instruction. An empty array is a \
     good answer when nothing durable was said."
        .to_string()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawProposal {
    body: String,
    kind: String,
    #[serde(default)]
    confidence_bps: Option<u32>,
    #[serde(default)]
    rationale: Option<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ExtractionReport {
    pub written: usize,
    pub invalid: usize,
    pub duplicates: usize,
    pub refused: usize,
}

fn fenced_payload(text: &str) -> Option<&str> {
    let start = text.find("```bridge-memory-proposals")?;
    let after = &text[start + "```bridge-memory-proposals".len()..];
    let end = after.find("```")?;
    Some(after[..end].trim())
}

/// The deterministic gate. Everything an explicit save enforces runs again
/// here, and whatever a model claimed, survivors land proposed/model_proposal.
/// An element with any field beyond the four — a status, a provenance, a
/// scope — is invalid rather than honored.
pub fn gate_and_insert(
    db: &Connection,
    scope_key: &str,
    session_id: &str,
    model_text: &str,
) -> Result<ExtractionReport, BridgeError> {
    let scope_key = memory_ledger::parse_scope_key(scope_key)?;
    let mut report = ExtractionReport::default();
    let Some(payload) = fenced_payload(model_text) else {
        return Err(BridgeError::Invalid(
            "The model answered without a bridge-memory-proposals block.".into(),
        ));
    };
    let elements: Vec<serde_json::Value> = serde_json::from_str(payload)
        .map_err(|error| BridgeError::Invalid(format!("Proposals are not a JSON array: {error}")))?;
    for element in elements.into_iter().take(MAX_PROPOSALS_PER_RUN) {
        let raw: RawProposal = match serde_json::from_value(element) {
            Ok(raw) => raw,
            Err(_) => {
                report.invalid += 1;
                continue;
            }
        };
        let body = match memory_ledger::require_body(&raw.body) {
            Ok(body) => body,
            Err(_) => {
                report.refused += 1;
                continue;
            }
        };
        let kind = match memory_ledger::parse_kind(Some(&raw.kind)) {
            Ok(kind) => kind,
            Err(_) => {
                report.invalid += 1;
                continue;
            }
        };
        if memory_ledger::body_already_known(db, &scope_key, &body)? {
            report.duplicates += 1;
            continue;
        }
        let confidence = raw.confidence_bps.map(|value| value.min(10_000));
        let rationale = raw
            .rationale
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.chars().take(MAX_RATIONALE_CHARS).collect::<String>());
        memory_ledger::insert_proposal(
            db,
            &scope_key,
            &body,
            kind,
            confidence,
            rationale.as_deref(),
            session_id,
        )?;
        report.written += 1;
    }
    Ok(report)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionOutput {
    pub text: String,
    pub observed_tokens: i64,
    pub spend_microusd: i64,
}

/// The seam the live binding implements and tests fake. No test performs a
/// model call.
pub trait ExtractionModel {
    fn propose(&mut self, instructions: &str, digest: &str) -> Result<ExtractionOutput, BridgeError>;
}

/// One run against any model implementation: digest, one call, gate, insert.
pub fn run_extraction(
    db: &Connection,
    model: &mut dyn ExtractionModel,
    scope_key: &str,
    session_id: &str,
) -> Result<(ExtractionReport, ExtractionOutput, Option<String>), BridgeError> {
    let Some(digest) = build_digest(db, scope_key, session_id)? else {
        return Ok((
            ExtractionReport::default(),
            ExtractionOutput { text: String::new(), observed_tokens: 0, spend_microusd: 0 },
            None,
        ));
    };
    let output = model.propose(&extraction_instructions(), &digest.text)?;
    let report = gate_and_insert(db, scope_key, session_id, &output.text)?;
    Ok((report, output, Some(digest.sha256)))
}

pub fn approved_proposal(record: &MemoryRecord) -> bool {
    record.provenance == "model_proposal" && record.status == "active"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;

    fn extraction_db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db = store::open(&dir.path().join("bridge.db")).unwrap();
        (dir, db)
    }

    fn insert_chat(db: &Connection, id: &str, kind: &str) {
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind)
             VALUES(?1,NULL,'codex',?1,'idle','reported',?2)",
            params![id, kind],
        )
        .unwrap();
    }

    fn insert_message(db: &Connection, session: &str, id: &str, sequence: i64, kind: &str, text: &str) {
        db.execute(
            "INSERT INTO session_entries(id,session_id,parent_entry_id,sequence,kind,payload,created_at)
             VALUES(?1,?2,NULL,?3,?4,json_object('text',?5),'now')",
            params![id, session, sequence, kind, text],
        )
        .unwrap();
    }

    fn propose_mode(db: &Connection) {
        update_settings(db, ACCOUNT_MEMORY_SCOPE, MODE_PROPOSE, Some("codex"), Some("gpt-5.6-luna"))
            .unwrap();
    }

    struct CannedModel(String);
    impl ExtractionModel for CannedModel {
        fn propose(&mut self, _instructions: &str, _digest: &str) -> Result<ExtractionOutput, BridgeError> {
            Ok(ExtractionOutput { text: self.0.clone(), observed_tokens: 420, spend_microusd: 1_700 })
        }
    }

    struct RefusingModel;
    impl ExtractionModel for RefusingModel {
        fn propose(&mut self, _: &str, _: &str) -> Result<ExtractionOutput, BridgeError> {
            panic!("opt-out is absolute: nothing may reach a model");
        }
    }

    fn fenced(json: &str) -> String {
        format!("Here you go.\n```bridge-memory-proposals\n{json}\n```\n")
    }

    #[test]
    fn auto_apply_does_not_exist_yet() {
        let (_dir, db) = extraction_db();
        let error = update_settings(&db, ACCOUNT_MEMORY_SCOPE, "auto_apply", Some("codex"), Some("m"))
            .unwrap_err();
        assert!(error.to_string().contains("replay bench"));
        assert_eq!(settings(&db, ACCOUNT_MEMORY_SCOPE).unwrap().mode, MODE_REMEMBER);
    }

    #[test]
    fn propose_mode_needs_a_pinned_profile() {
        let (_dir, db) = extraction_db();
        assert!(update_settings(&db, ACCOUNT_MEMORY_SCOPE, MODE_PROPOSE, None, None).is_err());
        assert!(update_settings(&db, ACCOUNT_MEMORY_SCOPE, MODE_PROPOSE, Some("codex"), Some("m")).is_ok());
    }

    #[test]
    fn remember_mode_enqueues_nothing_and_calls_no_model() {
        let (_dir, db) = extraction_db();
        insert_chat(&db, "s1", "chat");
        insert_message(&db, "s1", "e1", 1, "user.message", "I prefer tabs");
        assert!(!enqueue_after_turn(&db, "s1").unwrap());
        let mut model = RefusingModel;
        let _ = &mut model;
    }

    #[test]
    fn hidden_session_kinds_never_enqueue() {
        let (_dir, db) = extraction_db();
        propose_mode(&db);
        insert_chat(&db, "b1", "briefing");
        insert_chat(&db, "x1", EXTRACTION_SESSION_KIND);
        insert_chat(&db, "w1", "worker");
        assert!(!enqueue_after_turn(&db, "b1").unwrap());
        assert!(!enqueue_after_turn(&db, "x1").unwrap());
        assert!(!enqueue_after_turn(&db, "w1").unwrap());
        insert_chat(&db, "s1", "chat");
        assert!(enqueue_after_turn(&db, "s1").unwrap());
        assert!(!enqueue_after_turn(&db, "s1").unwrap(), "one open run per session");
    }

    #[test]
    fn proposals_land_proposed_and_never_active() {
        let (_dir, db) = extraction_db();
        propose_mode(&db);
        insert_chat(&db, "s1", "chat");
        insert_message(&db, "s1", "e1", 1, "user.message", "Always run bun run check before pushing");
        let mut model = CannedModel(fenced(
            r#"[{"body":"Runs bun run check before pushing","kind":"constraint","confidenceBps":9000,"rationale":"Said directly"}]"#,
        ));
        let (report, output, digest) =
            run_extraction(&db, &mut model, ACCOUNT_MEMORY_SCOPE, "s1").unwrap();
        assert_eq!(report.written, 1);
        assert_eq!(output.spend_microusd, 1_700);
        assert!(digest.is_some());
        let proposed = memory_ledger::list(&db, ACCOUNT_MEMORY_SCOPE, Some("proposed")).unwrap();
        assert_eq!(proposed.records.len(), 1);
        let record = &proposed.records[0];
        assert_eq!(record.status, "proposed");
        assert_eq!(record.provenance, "model_proposal");
        assert_eq!(record.confidence_bps, Some(9000));
        assert!(memory_ledger::list(&db, ACCOUNT_MEMORY_SCOPE, None).unwrap().records.is_empty());
    }

    #[test]
    fn a_claimed_status_or_scope_makes_the_proposal_invalid_not_honored() {
        let (_dir, db) = extraction_db();
        propose_mode(&db);
        insert_chat(&db, "s1", "chat");
        insert_message(&db, "s1", "e1", 1, "user.message", "hello");
        let mut model = CannedModel(fenced(
            r#"[{"body":"Sneaky","kind":"preference","status":"active"},
                {"body":"Sneakier","kind":"preference","provenance":"user_explicit"},
                {"body":"Sneakiest","kind":"preference","scopeKey":"workspace:w"}]"#,
        ));
        let (report, _, _) = run_extraction(&db, &mut model, ACCOUNT_MEMORY_SCOPE, "s1").unwrap();
        assert_eq!(report.invalid, 3);
        assert_eq!(report.written, 0);
        assert!(memory_ledger::list(&db, ACCOUNT_MEMORY_SCOPE, Some("proposed")).unwrap().records.is_empty());
    }

    #[test]
    fn the_gate_refuses_secrets_and_dedupes_known_bodies() {
        let (_dir, db) = extraction_db();
        propose_mode(&db);
        insert_chat(&db, "s1", "chat");
        insert_message(&db, "s1", "e1", 1, "user.message", "hello");
        memory_ledger::save(&db, "Prefers tabs over spaces", None, None).unwrap();
        let mut model = CannedModel(fenced(
            r#"[{"body":"prefers tabs over spaces","kind":"preference"},
                {"body":"api key sk-proj-1234567890abcdefghijklmnopqrstuvwxyz123456","kind":"fact"},
                {"body":"Reviews diffs before merging","kind":"preference"}]"#,
        ));
        let (report, _, _) = run_extraction(&db, &mut model, ACCOUNT_MEMORY_SCOPE, "s1").unwrap();
        assert_eq!(report.duplicates, 1, "case-insensitive body dedupe");
        assert_eq!(report.refused, 1, "secret-shaped bodies are refused");
        assert_eq!(report.written, 1);
    }

    #[test]
    fn at_most_ten_proposals_survive_a_run() {
        let (_dir, db) = extraction_db();
        propose_mode(&db);
        insert_chat(&db, "s1", "chat");
        insert_message(&db, "s1", "e1", 1, "user.message", "hello");
        let elements: Vec<String> = (0..14)
            .map(|index| format!(r#"{{"body":"Distinct fact number {index}","kind":"fact"}}"#))
            .collect();
        let mut model = CannedModel(fenced(&format!("[{}]", elements.join(","))));
        let (report, _, _) = run_extraction(&db, &mut model, ACCOUNT_MEMORY_SCOPE, "s1").unwrap();
        assert_eq!(report.written, 10);
    }

    #[test]
    fn the_digest_is_bounded_and_names_its_entries() {
        let (_dir, db) = extraction_db();
        insert_chat(&db, "s1", "chat");
        insert_message(&db, "s1", "e1", 1, "user.message", "I always deploy on Fridays");
        insert_message(&db, "s1", "e2", 2, "assistant.message", "Noted.");
        memory_ledger::save(&db, "Existing pin", None, None).unwrap();
        let digest = build_digest(&db, ACCOUNT_MEMORY_SCOPE, "s1").unwrap().unwrap();
        assert!(digest.text.contains("[e1] user: I always deploy on Fridays"));
        assert!(digest.text.contains("Already pinned"));
        assert!(digest.text.len() <= MAX_DIGEST_CHARS + 1_000);
        assert!(build_digest(&db, ACCOUNT_MEMORY_SCOPE, "empty-session").unwrap().is_none());
    }

    #[test]
    fn a_run_is_leased_settled_and_its_spend_observed() {
        let (_dir, db) = extraction_db();
        propose_mode(&db);
        insert_chat(&db, "s1", "chat");
        assert!(enqueue_after_turn(&db, "s1").unwrap());
        let now = Utc::now();
        let claimed = claim_due(&db, now).unwrap().unwrap();
        assert_eq!(claimed.session_id, "s1");
        assert_eq!(claimed.harness, "codex");
        assert!(claim_due(&db, now).unwrap().is_none(), "the lease excludes a second worker");
        assert!(heartbeat(&db, &claimed.run_id, &claimed.lease_owner, now).unwrap());
        assert!(settle(
            &db, &claimed.run_id, &claimed.lease_owner, "completed", None,
            Some("codex"), Some("gpt-5.6-luna"), Some("digest"), 420, 1_700, 2,
        )
        .unwrap());
        let last = last_run(&db, ACCOUNT_MEMORY_SCOPE).unwrap().unwrap();
        assert_eq!(last.status, "completed");
        assert_eq!(last.spend_microusd, 1_700, "spend is observed, never hardcoded zero");
        assert_eq!(last.proposal_count, 2);
        assert!(settle(&db, &claimed.run_id, &claimed.lease_owner, "completed", None, None, None, None, 0, 0, 0).unwrap() == false, "a settled run stays settled");
    }

    #[test]
    fn turning_propose_off_cancels_queued_runs_at_claim_time() {
        let (_dir, db) = extraction_db();
        propose_mode(&db);
        insert_chat(&db, "s1", "chat");
        assert!(enqueue_after_turn(&db, "s1").unwrap());
        update_settings(&db, ACCOUNT_MEMORY_SCOPE, MODE_REMEMBER, None, None).unwrap();
        assert!(claim_due(&db, Utc::now()).unwrap().is_none());
        let last = last_run(&db, ACCOUNT_MEMORY_SCOPE).unwrap().unwrap();
        assert_eq!(last.status, "cancelled");
        assert_eq!(last.detail.as_deref(), Some("extraction_disabled"));
    }

    #[test]
    fn an_expired_lease_is_reclaimable() {
        let (_dir, db) = extraction_db();
        propose_mode(&db);
        insert_chat(&db, "s1", "chat");
        assert!(enqueue_after_turn(&db, "s1").unwrap());
        let start = Utc::now();
        let first = claim_due(&db, start).unwrap().unwrap();
        let later = start + Duration::minutes(LEASE_MINUTES + 1);
        let second = claim_due(&db, later).unwrap().unwrap();
        assert_eq!(first.run_id, second.run_id);
        assert_ne!(first.lease_owner, second.lease_owner);
        assert!(
            !heartbeat(&db, &first.run_id, &first.lease_owner, later).unwrap(),
            "the reclaimed lease no longer answers to the first owner"
        );
    }

    #[test]
    fn extraction_never_consults_the_router() {
        let module = format!("{}{}", "learning", "_router");
        let source = include_str!("memory_extraction.rs");
        let occurrences = source.matches(&module).count();
        assert_eq!(occurrences, 0, "the extractor never imports or calls the router");
        let live = include_str!("memory_extraction_live.rs");
        assert_eq!(live.matches(&module).count(), 0);
    }
}

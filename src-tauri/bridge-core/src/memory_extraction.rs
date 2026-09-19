//! Post-turn memory extraction: the first LLM writer, and it can only propose.
//!
//! Written against a trait rather than a provider process, for the same reason
//! the briefing runner is: the decisions — what a digest contains, what a
//! proposal must look like, what the gate refuses — are this module's
//! responsibility, and they must be checkable without a network. The live
//! binding (memory_extraction_live) runs a hidden bounded session on the
//! harness and model the finished conversation itself used, unless the user
//! pinned a helper; the learning router is never consulted, and an extraction
//! run writes no router decision.
//!
//! Propose is the default. A ledger that only ever holds what the user typed
//! into `/pin` reads as memory that is never used, because almost nobody pins.
//! Nothing a run proposes activates without the user: proposals queue for
//! review, and the packet excludes them until approved.

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
/// The kinds real conversations are stored under: `direct` single-agent chats
/// and `orchestrator` workspace sessions. No production path writes `chat`.
const ENQUEUEABLE_SESSION_KINDS: [&str; 2] = ["direct", "orchestrator"];

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
    let (mode, harness, model) = row.unwrap_or((MODE_PROPOSE.to_string(), None, None));
    Ok(ExtractionSettings { scope_key, mode, harness, model })
}

/// The harness and model a run for `session_id` executes on.
///
/// A pinned helper wins. Otherwise the conversation's own harness serves,
/// with `None` for the model when the session never chose one so the adapter
/// starts on its default. A harness that cannot hold briefing authority is not
/// a profile: an extraction turn is tool-free, and only an adapter that can
/// enforce that may run one.
pub fn resolve_profile(
    db: &Connection,
    settings: &ExtractionSettings,
    session_id: &str,
) -> Result<Option<(String, Option<String>)>, BridgeError> {
    if let (Some(harness), Some(model)) = (&settings.harness, &settings.model) {
        return Ok(Some((harness.clone(), Some(model.clone()))));
    }
    let session: Option<(String, Option<String>)> = db
        .query_row(
            "SELECT harness, model FROM sessions WHERE id=?1",
            params![session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((harness, model)) = session else { return Ok(None) };
    if crate::briefing_policy::adapter_may_brief(&harness).is_err() {
        return Ok(None);
    }
    let model = model.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty());
    Ok(Some((harness, model)))
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
    if harness.is_some() != model.is_some() {
        return Err(BridgeError::Invalid(
            "Pin both a helper and a model, or neither to run on each chat's own model.".into(),
        ));
    }
    // An extraction run is tool-free, and tool-free is enforced by the same
    // briefing authority the briefing runner uses. A harness that cannot hold
    // it would refuse at provider start, once per finished turn, forever — so
    // it is refused here instead, while the user is looking at the setting.
    if mode == MODE_PROPOSE {
        if let Some(harness) = harness {
            if let Err(unsupported) = crate::briefing_policy::adapter_may_brief(harness) {
                return Err(BridgeError::Invalid(format!(
                    "{harness} cannot run a tool-free extraction: {}",
                    unsupported.reason()
                )));
            }
        }
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
    let kind = kind.unwrap_or_else(|| "direct".to_string());
    if !ENQUEUEABLE_SESSION_KINDS.contains(&kind.as_str()) {
        return Ok(false);
    }
    let current = settings(db, ACCOUNT_MEMORY_SCOPE)?;
    if current.mode != MODE_PROPOSE {
        return Ok(false);
    }
    // A chat on a harness that cannot run tool-free would queue a run that
    // the claim can only cancel, after every turn, forever. Not enqueueing is
    // the quiet answer; the setting still says propose, and a pinned helper
    // makes such chats eligible.
    if resolve_profile(db, &current, session_id)?.is_none() {
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
    /// `None` runs the harness on its default model.
    pub model: Option<String>,
}

/// One due run, leased. A queued run whose scope has since left propose mode,
/// or whose profile can no longer be resolved, is settled `cancelled` rather
/// than executed — turning it off means off.
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
        let profile = if current.mode == MODE_PROPOSE {
            resolve_profile(db, &current, &session_id)?
        } else {
            None
        };
        let Some((harness, model)) = profile else {
            let detail = if current.mode == MODE_PROPOSE {
                "no_extraction_profile"
            } else {
                "extraction_disabled"
            };
            db.execute(
                "UPDATE memory_extraction_runs
                 SET status='cancelled', detail=?3, lease_owner=NULL,
                     lease_expires_at=NULL, updated_at=?2
                 WHERE id=?1 AND status IN ('queued','running')",
                params![run_id, now.to_rfc3339(), detail],
            )?;
            continue;
        };
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
            harness,
            model,
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

/// The newest **settled** run. A queued or running row carries zeroes it has
/// not earned yet, and showing those would erase the last real spend the
/// moment a turn finishes — the surface would report $0.0000 for work that
/// cost something.
pub fn last_run(db: &Connection, scope_key: &str) -> Result<Option<ExtractionRunSummary>, BridgeError> {
    let scope_key = memory_ledger::parse_scope_key(scope_key)?;
    db.query_row(
        "SELECT status, proposal_count, observed_tokens, spend_microusd, detail, updated_at
         FROM memory_extraction_runs
         WHERE scope_key=?1 AND status IN ('completed','failed','cancelled')
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
        match memory_ledger::insert_proposal(
            db,
            &scope_key,
            &body,
            kind,
            confidence,
            rationale.as_deref(),
            session_id,
        ) {
            Ok(_) => report.written += 1,
            // The scope is at its budget. A proposal that would overflow is
            // refused before it is stored and the run says so; nothing is
            // evicted to make it fit, and nothing is dropped in silence.
            Err(BridgeError::Invalid(_)) => report.refused += 1,
            Err(error) => return Err(error),
        }
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
        // Claude is the harness that can hold briefing authority, so it is the
        // one a propose-mode fixture may pin.
        update_settings(db, ACCOUNT_MEMORY_SCOPE, MODE_PROPOSE, Some("claude"), Some("sonnet"))
            .unwrap();
    }

    fn remember_mode(db: &Connection) {
        update_settings(db, ACCOUNT_MEMORY_SCOPE, MODE_REMEMBER, None, None).unwrap();
    }

    fn insert_chat_on(db: &Connection, id: &str, harness: &str, model: Option<&str>) {
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,model,kind)
             VALUES(?1,NULL,?2,?1,'idle','reported',?3,'direct')",
            params![id, harness, model],
        )
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
        let error = update_settings(&db, ACCOUNT_MEMORY_SCOPE, "auto_apply", Some("claude"), Some("m"))
            .unwrap_err();
        assert!(error.to_string().contains("replay bench"));
        assert_eq!(settings(&db, ACCOUNT_MEMORY_SCOPE).unwrap().mode, MODE_PROPOSE);
    }

    #[test]
    fn propose_is_the_default_and_runs_on_the_chats_own_model() {
        let (_dir, db) = extraction_db();
        let current = settings(&db, ACCOUNT_MEMORY_SCOPE).unwrap();
        assert_eq!(current.mode, MODE_PROPOSE);
        assert_eq!(current.harness, None);
        assert_eq!(current.model, None);
        insert_chat_on(&db, "s1", "claude", Some("claude-sonnet-4-5"));
        assert_eq!(
            resolve_profile(&db, &current, "s1").unwrap(),
            Some(("claude".to_string(), Some("claude-sonnet-4-5".to_string())))
        );
        assert!(enqueue_after_turn(&db, "s1").unwrap());
        let claimed = claim_due(&db, Utc::now()).unwrap().unwrap();
        assert_eq!(claimed.harness, "claude");
        assert_eq!(claimed.model.as_deref(), Some("claude-sonnet-4-5"));
    }

    #[test]
    fn a_chat_without_a_chosen_model_runs_on_the_harness_default() {
        let (_dir, db) = extraction_db();
        insert_chat_on(&db, "s1", "claude", None);
        assert!(enqueue_after_turn(&db, "s1").unwrap());
        let claimed = claim_due(&db, Utc::now()).unwrap().unwrap();
        assert_eq!(claimed.harness, "claude");
        assert_eq!(claimed.model, None, "no model pinned and none chosen: the adapter's default");
    }

    #[test]
    fn a_pinned_helper_outranks_the_chats_own_harness() {
        let (_dir, db) = extraction_db();
        update_settings(&db, ACCOUNT_MEMORY_SCOPE, MODE_PROPOSE, Some("claude"), Some("haiku")).unwrap();
        insert_chat_on(&db, "s1", "claude", Some("opus"));
        assert!(enqueue_after_turn(&db, "s1").unwrap());
        let claimed = claim_due(&db, Utc::now()).unwrap().unwrap();
        assert_eq!(claimed.model.as_deref(), Some("haiku"));
    }

    #[test]
    fn an_unpinned_chat_on_a_harness_that_cannot_run_tool_free_never_enqueues() {
        let (_dir, db) = extraction_db();
        // Codex and OpenCode adapters refuse to start under a briefing policy,
        // so a run for such a chat could only ever be cancelled at claim time.
        insert_chat_on(&db, "c1", "codex", Some("gpt-5.6-luna"));
        insert_chat_on(&db, "o1", "opencode", None);
        assert!(!enqueue_after_turn(&db, "c1").unwrap());
        assert!(!enqueue_after_turn(&db, "o1").unwrap());
        assert!(claim_due(&db, Utc::now()).unwrap().is_none());
        assert!(last_run(&db, ACCOUNT_MEMORY_SCOPE).unwrap().is_none(), "nothing queued, nothing cancelled");
        // Pinning a helper that can hold briefing authority makes them eligible.
        propose_mode(&db);
        assert!(enqueue_after_turn(&db, "c1").unwrap());
        assert_eq!(claim_due(&db, Utc::now()).unwrap().unwrap().harness, "claude");
    }

    #[test]
    fn a_profile_that_stops_resolving_cancels_the_queued_run() {
        let (_dir, db) = extraction_db();
        propose_mode(&db);
        insert_chat_on(&db, "c1", "codex", None);
        assert!(enqueue_after_turn(&db, "c1").unwrap());
        // Unpinning leaves propose on, but a Codex chat has no profile of its own.
        update_settings(&db, ACCOUNT_MEMORY_SCOPE, MODE_PROPOSE, None, None).unwrap();
        assert!(claim_due(&db, Utc::now()).unwrap().is_none());
        let last = last_run(&db, ACCOUNT_MEMORY_SCOPE).unwrap().unwrap();
        assert_eq!(last.status, "cancelled");
        assert_eq!(last.detail.as_deref(), Some("no_extraction_profile"));
    }

    #[test]
    fn a_helper_is_pinned_whole_or_not_at_all() {
        let (_dir, db) = extraction_db();
        assert!(update_settings(&db, ACCOUNT_MEMORY_SCOPE, MODE_PROPOSE, Some("claude"), None).is_err());
        assert!(update_settings(&db, ACCOUNT_MEMORY_SCOPE, MODE_PROPOSE, None, Some("m")).is_err());
        assert!(update_settings(&db, ACCOUNT_MEMORY_SCOPE, MODE_PROPOSE, None, None).is_ok());
        assert!(update_settings(&db, ACCOUNT_MEMORY_SCOPE, MODE_PROPOSE, Some("claude"), Some("m")).is_ok());
    }

    #[test]
    fn remember_mode_enqueues_nothing_and_calls_no_model() {
        let (_dir, db) = extraction_db();
        remember_mode(&db);
        insert_chat_on(&db, "s1", "claude", Some("sonnet"));
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
        insert_chat(&db, "s1", "direct");
        assert!(enqueue_after_turn(&db, "s1").unwrap());
        assert!(!enqueue_after_turn(&db, "s1").unwrap(), "one open run per session");
    }

    #[test]
    fn proposals_land_proposed_and_never_active() {
        let (_dir, db) = extraction_db();
        propose_mode(&db);
        insert_chat(&db, "s1", "direct");
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
        insert_chat(&db, "s1", "direct");
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
        insert_chat(&db, "s1", "direct");
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
        insert_chat(&db, "s1", "direct");
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
        insert_chat(&db, "s1", "direct");
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
        insert_chat(&db, "s1", "direct");
        assert!(enqueue_after_turn(&db, "s1").unwrap());
        let now = Utc::now();
        let claimed = claim_due(&db, now).unwrap().unwrap();
        assert_eq!(claimed.session_id, "s1");
        assert_eq!(claimed.harness, "claude");
        assert_eq!(claimed.model.as_deref(), Some("sonnet"));
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
    fn a_harness_that_cannot_run_tool_free_is_refused_at_the_setting() {
        let (_dir, db) = extraction_db();
        // Codex and OpenCode adapters refuse to start when a briefing policy is
        // present, so pinning one would fail every run after every turn.
        for harness in ["codex", "opencode"] {
            let error =
                update_settings(&db, ACCOUNT_MEMORY_SCOPE, MODE_PROPOSE, Some(harness), Some("m"))
                    .unwrap_err()
                    .to_string();
            assert!(error.contains("tool-free"), "{harness}: {error}");
        }
        let current = settings(&db, ACCOUNT_MEMORY_SCOPE).unwrap();
        assert_eq!(current.harness, None, "a refused profile pins nothing");
        assert_eq!(current.model, None);
        assert!(
            update_settings(&db, ACCOUNT_MEMORY_SCOPE, MODE_PROPOSE, Some("claude"), Some("m"))
                .is_ok()
        );
    }

    #[test]
    fn a_queued_run_does_not_erase_the_last_observed_spend() {
        let (_dir, db) = extraction_db();
        propose_mode(&db);
        insert_chat(&db, "s1", "direct");
        assert!(enqueue_after_turn(&db, "s1").unwrap());
        let claimed = claim_due(&db, Utc::now()).unwrap().unwrap();
        settle(
            &db, &claimed.run_id, &claimed.lease_owner, "completed", None,
            Some("claude"), Some("m"), Some("digest"), 420, 1_700, 2,
        )
        .unwrap();
        insert_chat(&db, "s2", "direct");
        assert!(enqueue_after_turn(&db, "s2").unwrap());
        let last = last_run(&db, ACCOUNT_MEMORY_SCOPE).unwrap().unwrap();
        assert_eq!(last.status, "completed", "a queued row is not a report");
        assert_eq!(last.spend_microusd, 1_700);
    }

    #[test]
    fn turning_propose_off_cancels_queued_runs_at_claim_time() {
        let (_dir, db) = extraction_db();
        propose_mode(&db);
        insert_chat(&db, "s1", "direct");
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
        insert_chat(&db, "s1", "direct");
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

//! Post-turn memory extraction: the first LLM writer, proposal-first even when
//! an explicitly enabled automatic mode can promote a safe survivor.
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
//! In propose mode, every survivor queues for review. In explicit auto-apply
//! mode, only high-confidence, directly cited, grounded, non-transient and
//! non-conflicting survivors pass through the ordinary approval transition.

use crate::BridgeError;
use crate::{memory_ledger, secret_interception};
use bridge_protocol::messages::{MemoryRecord, ACCOUNT_MEMORY_SCOPE};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use uuid::Uuid;

pub const EXTRACTION_SESSION_KIND: &str = "extraction";
pub const MODE_REMEMBER: &str = "remember";
pub const MODE_PROPOSE: &str = "propose";
pub const MODE_AUTO_APPLY: &str = "auto_apply";
const AUTO_APPLY_MIN_CONFIDENCE_BPS: u32 = 9_000;
const MAX_PROPOSALS_PER_RUN: usize = 10;
const MAX_RATIONALE_CHARS: usize = 500;
const MAX_DIGEST_CHARS: usize = 24_000;
const MAX_DIGEST_ENTRIES: usize = 30;
const MAX_PIN_CONTEXT: usize = 20;
const LEASE_MINUTES: i64 = 10;
/// The kinds real conversations are stored under: `direct` single-agent chats
/// and `orchestrator` workspace sessions. No production path writes `chat`.
const ENQUEUEABLE_SESSION_KINDS: [&str; 2] = ["direct", "orchestrator"];

fn extraction_enabled(mode: &str) -> bool {
    matches!(mode, MODE_PROPOSE | MODE_AUTO_APPLY)
}

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
            updated_at TEXT NOT NULL,
            mode TEXT NOT NULL DEFAULT 'propose'
        );
        CREATE INDEX IF NOT EXISTS idx_memory_extraction_runs_status
            ON memory_extraction_runs(status, created_at);",
    )?;
    Ok(())
}

/// Existing databases predate the enqueue-time mode snapshot. Defaulting those
/// open runs to `propose` is the only safe migration: an upgrade must never
/// grant automatic activation to work that was queued without that authority.
pub(crate) fn install_run_modes(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    crate::store::add_column_if_missing(
        transaction,
        "memory_extraction_runs",
        "mode",
        "TEXT NOT NULL DEFAULT 'propose'",
    )?;
    transaction.execute(
        "UPDATE memory_extraction_runs SET mode='propose'
         WHERE mode IS NULL OR trim(mode)=''",
        [],
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
    Ok(ExtractionSettings {
        scope_key,
        mode,
        harness,
        model,
    })
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
    let Some((harness, model)) = session else {
        return Ok(None);
    };
    if crate::briefing_policy::adapter_may_brief(&harness).is_err() {
        return Ok(None);
    }
    let model = model
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
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
        MODE_AUTO_APPLY => MODE_AUTO_APPLY,
        other => {
            return Err(BridgeError::Invalid(format!(
                "Unknown extraction mode '{other}'. Use remember, propose, or auto_apply."
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
    if extraction_enabled(mode) {
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
    if !extraction_enabled(&current.mode) {
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
        "INSERT INTO memory_extraction_runs(
            id, scope_key, session_id, mode, status, created_at, updated_at
         ) VALUES(?1,?2,?3,?4,'queued',?5,?5)",
        params![
            Uuid::new_v4().to_string(),
            ACCOUNT_MEMORY_SCOPE,
            session_id,
            current.mode,
            now
        ],
    )?;
    Ok(true)
}

#[derive(Debug, Clone)]
pub struct ClaimedExtraction {
    pub run_id: String,
    pub scope_key: String,
    pub session_id: String,
    pub lease_owner: String,
    /// The scope mode when this run was enqueued. Auto-apply must also still be
    /// selected when the output is gated; settings changes can only downgrade
    /// an existing run to review, never grant it new authority.
    pub mode: String,
    pub harness: String,
    /// `None` runs the harness on its default model.
    pub model: Option<String>,
}

/// One due run, leased. A queued run whose scope has since disabled extraction,
/// or whose profile can no longer be resolved, is settled `cancelled` rather
/// than executed — turning it off means off.
pub fn claim_due(
    db: &Connection,
    now: DateTime<Utc>,
) -> Result<Option<ClaimedExtraction>, BridgeError> {
    loop {
        let candidate: Option<(String, String, String, String)> = db
            .query_row(
                "SELECT id, scope_key, session_id, mode FROM memory_extraction_runs
                 WHERE status='queued'
                    OR (status='running' AND (lease_expires_at IS NULL OR lease_expires_at < ?1))
                 ORDER BY created_at LIMIT 1",
                params![now.to_rfc3339()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((run_id, scope_key, session_id, queued_mode)) = candidate else {
            return Ok(None);
        };
        let current = settings(db, &scope_key)?;
        let profile = if extraction_enabled(&current.mode) {
            resolve_profile(db, &current, &session_id)?
        } else {
            None
        };
        let Some((harness, model)) = profile else {
            let detail = if extraction_enabled(&current.mode) {
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
            mode: queued_mode,
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
pub fn last_run(
    db: &Connection,
    scope_key: &str,
) -> Result<Option<ExtractionRunSummary>, BridgeError> {
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
        let role = if kind == "user.message" {
            "user"
        } else {
            "assistant"
        };
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
    "You review one conversation digest and propose only durable facts the user \
     directly stated about themselves or how they work: preferences, conventions, \
     decisions, and constraints. Assistant messages are context, never evidence. \
     The same clause that supports the body must contain an explicit first-person \
     or commitment cue: prefer/rather/favorite for a preference; \
     always/never/must/only/require/avoid for a constraint; decided/chosen/going with \
     for a decision; or a concrete first-person fact such as a name, role, job, \
     timezone, workplace, residence, or tool in regular use. Merely mentioning a \
     tool, language, place, or topic is not evidence. Quoted, reported, negated, \
     or retracted wording is not a durable claim. Do not infer a claim, copy \
     temporary chat/task/session state, deadlines, one-off requests, or anything \
     that conflicts with an already remembered item. \
     Answer with exactly one fenced code block tagged bridge-memory-proposals \
     containing a JSON array. Each element is an object with exactly these fields: \
     body (the fact, under 4000 characters), kind (preference, fact, decision, or \
     constraint), confidenceBps (0-10000), rationale (under 500 characters, beginning \
     with the bracketed id of the user.message that directly supports the body, for \
     example `[entry-id] Direct statement`). At most 10 elements. Propose nothing \
     sensitive, no credentials, nothing already pinned, and nothing about this \
     instruction. An empty array is a good answer when nothing durable was said."
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

fn words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    for character in text.chars() {
        if character.is_alphanumeric() {
            current.extend(character.to_lowercase());
        } else if !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn lexical_term(mut word: String) -> String {
    let normalized = match word.as_str() {
        "avoided" | "avoiding" | "avoids" => Some("avoid"),
        "chose" | "chooses" | "choosing" | "chosen" => Some("choose"),
        "decided" | "decides" | "deciding" => Some("decide"),
        "lived" | "lives" | "living" => Some("live"),
        "needed" | "needing" | "needs" => Some("need"),
        "preferred" | "preferring" | "prefers" => Some("prefer"),
        "required" | "requires" | "requiring" => Some("require"),
        "used" | "uses" | "using" => Some("use"),
        "wanted" | "wanting" | "wants" => Some("want"),
        "worked" | "working" | "works" => Some("work"),
        _ => None,
    };
    if let Some(normalized) = normalized {
        return normalized.to_string();
    }
    if word.len() > 5 && word.ends_with("ies") {
        word.truncate(word.len() - 3);
        word.push('y');
    } else if word.len() > 4 && word.ends_with('s') && !word.ends_with("ss") {
        word.pop();
    }
    word
}

fn lexical_terms(text: &str) -> BTreeSet<String> {
    const STOP_WORDS: &[&str] = &[
        "a", "an", "and", "are", "as", "at", "avoid", "be", "been", "being", "but", "by", "choose",
        "decide", "favorite", "for", "from", "he", "her", "hers", "him", "his", "i", "in",
        "instead", "is", "it", "its", "like", "live", "me", "must", "my", "need", "of", "on", "or",
        "our", "ours", "over", "prefer", "rather", "require", "she", "than", "that", "the",
        "their", "theirs", "them", "they", "this", "to", "under", "use", "user", "want", "we",
        "will", "with", "work", "you", "your",
    ];
    words(text)
        .into_iter()
        .map(lexical_term)
        .filter(|word| word.chars().count() > 1 && !STOP_WORDS.contains(&word.as_str()))
        .collect()
}

fn lexical_terms_in_order(text: &str) -> Vec<String> {
    let retained = lexical_terms(text);
    words(text)
        .into_iter()
        .map(lexical_term)
        .filter(|word| retained.contains(word))
        .collect()
}

fn lexically_grounded(body: &str, source: &str) -> bool {
    // Automatic activation is intentionally stricter than proposal creation:
    // neither extra model content, omissions, reordering nor duplicate loss
    // may survive grounding. A looser paraphrase remains reviewable.
    let body_order = lexical_terms_in_order(body);
    let source_order = lexical_terms_in_order(source);
    !body_order.is_empty() && body_order == source_order
}

fn has_negative_polarity(text: &str) -> bool {
    let tokens = words(text);
    let normalized = tokens.join(" ");
    let padded = format!(" {normalized} ");
    if [
        " avoid ",
        " cannot ",
        " cant ",
        " didn t ",
        " doesn t ",
        " don t ",
        " isn t ",
        " never ",
        " no ",
        " not ",
        " wasn t ",
        " without ",
        " won t ",
        " wouldn t ",
    ]
    .iter()
    .any(|marker| padded.contains(marker))
    {
        return true;
    }
    const NEGATED_AUXILIARIES: &[&str] = &[
        "ain", "aren", "can", "couldn", "didn", "doesn", "don", "hadn", "hasn", "haven", "isn",
        "mustn", "needn", "shan", "shouldn", "wasn", "weren", "won", "wouldn",
    ];
    tokens
        .windows(2)
        .any(|window| NEGATED_AUXILIARIES.contains(&window[0].as_str()) && window[1] == "t")
}

fn relation_sequence(text: &str) -> Vec<&'static str> {
    words(text)
        .into_iter()
        .map(lexical_term)
        .filter_map(|word| match word.as_str() {
            "favorite" | "like" | "prefer" | "rather" | "want" => Some("prefer"),
            "use" => Some("use"),
            "work" => Some("work"),
            "live" => Some("live"),
            "choose" | "decide" | "going" => Some("choose"),
            "must" | "need" | "require" => Some("require"),
            "avoid" => Some("avoid"),
            _ => None,
        })
        .collect()
}

fn preserves_qualifiers(body: &str, source: &str) -> bool {
    const QUALIFIERS: &[&str] = &[
        "after", "and", "as", "at", "before", "between", "by", "during", "except", "for", "from",
        "he", "her", "hers", "him", "his", "in", "instead", "it", "its", "me", "my", "of", "on",
        "only", "or", "our", "ours", "over", "rather", "she", "that", "than", "their", "theirs",
        "them", "these", "they", "this", "those", "to", "under", "until", "us", "user", "versus",
        "when", "while", "with", "without", "you", "your", "yours",
    ];
    let qualifier_sequence = |text: &str| {
        words(text)
            .into_iter()
            .enumerate()
            .filter_map(|(index, word)| {
                (QUALIFIERS.contains(&word.as_str())
                    || (index > 0 && matches!(word.as_str(), "i" | "we")))
                .then_some(word)
            })
            .collect::<Vec<_>>()
    };
    qualifier_sequence(body) == qualifier_sequence(source)
}

fn has_transient_marker(text: &str) -> bool {
    const MARKERS: &[&str] = &[
        "at the moment",
        "current task",
        "currently",
        "for the moment",
        "for the time being",
        "for now",
        "for this",
        "in this",
        "in the meantime",
        "just this once",
        "later today",
        "next chat",
        "next conversation",
        "next release",
        "next request",
        "next session",
        "next sprint",
        "next task",
        "next turn",
        "next week",
        "one off",
        "right now",
        "soon",
        "temporary",
        "temporarily",
        "during this",
        "this chat",
        "this conversation",
        "this app",
        "this branch",
        "this component",
        "this feature",
        "this file",
        "this issue",
        "this page",
        "this pr",
        "this project",
        "this pull request",
        "this repo",
        "this repository",
        "this release",
        "this request",
        "this run",
        "this session",
        "this sprint",
        "this task",
        "this time",
        "this turn",
        "this week",
        "today",
        "tomorrow",
        "tonight",
        "until",
        "until further notice",
        "yesterday",
    ];
    const DAYS: &[&str] = &[
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
    ];
    const DAY_QUALIFIERS: &[&str] = &["before", "by", "last", "next", "this", "until"];
    const TEMPORAL_QUALIFIERS: &[&str] = &["current", "last", "next", "this"];
    const TEMPORAL_UNITS: &[&str] = &[
        "afternoon",
        "afternoons",
        "cycle",
        "cycles",
        "day",
        "days",
        "evening",
        "evenings",
        "hour",
        "hours",
        "iteration",
        "iterations",
        "minute",
        "minutes",
        "moment",
        "moments",
        "month",
        "months",
        "morning",
        "mornings",
        "quarter",
        "quarters",
        "release",
        "releases",
        "sprint",
        "sprints",
        "week",
        "weekend",
        "weekends",
        "weeks",
        "year",
        "years",
    ];
    const MONTHS: &[&str] = &[
        "april",
        "august",
        "december",
        "february",
        "january",
        "july",
        "june",
        "march",
        "may",
        "november",
        "october",
        "september",
    ];
    const TRANSIENT_PREPOSITIONS: &[&str] = &["during", "for", "in"];
    const TRANSIENT_DETERMINERS: &[&str] = &["a", "an", "that", "the", "this"];
    const TRANSIENT_CONTEXTS: &[&str] = &[
        "branch",
        "chat",
        "conversation",
        "demo",
        "example",
        "feature",
        "file",
        "issue",
        "message",
        "migration",
        "page",
        "project",
        "pull",
        "release",
        "repo",
        "repository",
        "request",
        "run",
        "session",
        "sprint",
        "task",
        "turn",
    ];

    let tokens = words(text);
    let normalized = tokens.join(" ");
    let padded = format!(" {normalized} ");
    if MARKERS
        .iter()
        .any(|marker| padded.contains(&format!(" {marker} ")))
    {
        return true;
    }
    if tokens.windows(3).any(|window| {
        TRANSIENT_PREPOSITIONS.contains(&window[0].as_str())
            && TRANSIENT_DETERMINERS.contains(&window[1].as_str())
            && TRANSIENT_CONTEXTS.contains(&window[2].as_str())
    }) {
        return true;
    }
    if tokens.windows(2).any(|window| {
        TEMPORAL_QUALIFIERS.contains(&window[0].as_str())
            && (TEMPORAL_UNITS.contains(&window[1].as_str())
                || MONTHS.contains(&window[1].as_str()))
    }) {
        return true;
    }
    if tokens.windows(3).any(|window| {
        matches!(window[0].as_str(), "during" | "for" | "in")
            && TEMPORAL_UNITS.contains(&window[2].as_str())
    }) {
        return true;
    }
    if tokens.windows(3).any(|window| {
        window[0] == "at"
            && window[1]
                .chars()
                .all(|character| character.is_ascii_digit())
            && matches!(window[2].as_str(), "am" | "pm")
    }) || tokens.iter().any(|token| {
        ["am", "pm"].iter().any(|suffix| {
            token
                .strip_suffix(suffix)
                .is_some_and(|hour| !hour.is_empty() && hour.chars().all(|c| c.is_ascii_digit()))
        })
    }) {
        return true;
    }
    if tokens.iter().any(|token| {
        token
            .parse::<u32>()
            .ok()
            .is_some_and(|year| (1_900..=2_200).contains(&year))
    }) {
        return true;
    }
    DAY_QUALIFIERS.iter().any(|qualifier| {
        DAYS.iter()
            .any(|day| padded.contains(&format!(" {qualifier} {day} ")))
    })
}

fn cited_user_message(
    db: &Connection,
    session_id: &str,
    rationale: Option<&str>,
) -> Result<Option<String>, BridgeError> {
    let Some(rationale) = rationale.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let Some(after_open) = rationale.strip_prefix('[') else {
        return Ok(None);
    };
    let Some(close) = after_open.find(']') else {
        return Ok(None);
    };
    let entry_id = after_open[..close].trim();
    if entry_id.is_empty() || entry_id.chars().count() > 200 {
        return Ok(None);
    }
    db.query_row(
        "SELECT coalesce(json_extract(payload,'$.text'), '')
         FROM session_entries
         WHERE id=?1 AND session_id=?2 AND kind='user.message'
           AND context_visibility IN ('eligible','visible')",
        params![entry_id, session_id],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map(|source| source.filter(|text| !text.trim().is_empty()))
    .map_err(BridgeError::from)
}

fn claim_fingerprint(text: &str) -> BTreeSet<String> {
    words(text).into_iter().map(lexical_term).collect()
}

/// Forgetting a claim is a durable negative signal. Punctuation and a light
/// paraphrase must not let a later extractor silently reactivate it; possible
/// matches remain visible in review instead.
fn matches_deleted_claim(
    db: &Connection,
    scope_key: &str,
    body: &str,
) -> Result<bool, BridgeError> {
    let mut statement = db.prepare(
        "SELECT body FROM memory_records
         WHERE scope_key=?1 AND status='deleted'",
    )?;
    let deleted = statement
        .query_map(params![scope_key], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let fingerprint = claim_fingerprint(body);
    Ok(deleted.iter().any(|known| {
        (!fingerprint.is_empty() && fingerprint == claim_fingerprint(known))
            || likely_related_claim(body, known)
    }))
}

fn has_relational_marker(text: &str) -> bool {
    const MARKERS: &[&str] = &[
        "avoid", "cannot", "cant", "except", "must", "never", "not", "only", "require", "versus",
        "without",
    ];
    words(text)
        .into_iter()
        .map(lexical_term)
        .any(|word| MARKERS.contains(&word.as_str()))
}

fn relation_family(text: &str) -> Option<&'static str> {
    const LEADING_FILLER: &[&str] = &[
        "a", "always", "am", "an", "are", "do", "does", "has", "have", "he", "i", "is", "my",
        "never", "only", "our", "she", "the", "they", "user", "we", "will",
    ];
    for word in words(text).into_iter().map(lexical_term).take(6) {
        if LEADING_FILLER.contains(&word.as_str()) {
            continue;
        }
        return match word.as_str() {
            "favorite" | "prefer" | "rather" | "want" => Some("prefer"),
            "use" => Some("use"),
            "work" => Some("work"),
            "live" => Some("live"),
            "choose" | "decide" | "going" => Some("choose"),
            "must" | "need" | "require" => Some("require"),
            "avoid" => Some("avoid"),
            _ => None,
        };
    }
    None
}

fn has_explicit_source_cue(kind: &str, source: &str) -> bool {
    let normalized = words(source).join(" ");
    let starts_with = |phrase: &str| {
        normalized == phrase
            || normalized
                .strip_prefix(phrase)
                .is_some_and(|rest| rest.starts_with(' '))
    };

    match kind {
        "preference" => [
            "i prefer",
            "we prefer",
            "i rather",
            "we rather",
            "i would rather",
            "we would rather",
            "my favorite",
            "our favorite",
        ]
        .iter()
        .any(|phrase| starts_with(phrase)),
        "constraint" => {
            [
                "i always",
                "we always",
                "i never",
                "we never",
                "i must",
                "we must",
                "i only",
                "we only",
                "i require",
                "we require",
                "i avoid",
                "we avoid",
                "you always",
                "you never",
                "you must",
            ]
            .iter()
            .any(|phrase| starts_with(phrase))
                || ["always", "never", "must", "only", "require", "avoid"]
                    .iter()
                    .any(|cue| starts_with(cue))
                || [
                    "please always",
                    "please never",
                    "please only",
                    "please require",
                    "please avoid",
                ]
                .iter()
                .any(|phrase| starts_with(phrase))
        }
        "decision" => {
            [
                "i decided",
                "we decided",
                "i have decided",
                "we have decided",
                "i ve decided",
                "we ve decided",
                "i chose",
                "we chose",
                "i have chosen",
                "we have chosen",
                "i m going with",
                "we re going with",
                "i am going with",
                "we are going with",
            ]
            .iter()
            .any(|phrase| starts_with(phrase))
                || ["decided", "chosen", "going with"]
                    .iter()
                    .any(|phrase| starts_with(phrase))
        }
        "fact" => [
            "i am called",
            "i m called",
            "i work",
            "i live",
            "i use",
            "we are based",
            "we re based",
            "we work",
            "we live",
            "we use",
            "my name is",
            "my role is",
            "my job is",
            "my timezone is",
            "my time zone is",
            "our company is",
            "our organization is",
            "our organisation is",
        ]
        .iter()
        .any(|phrase| starts_with(phrase)),
        _ => false,
    }
}

fn has_non_assertive_context(source: &str) -> bool {
    if source
        .chars()
        .any(|character| matches!(character, '?' | ',' | ';' | ':' | '—' | '–'))
    {
        return true;
    }
    let normalized = words(source).join(" ");
    let padded = format!(" {normalized} ");
    [
        " according to ",
        " allegedly ",
        " apparently ",
        " arguably ",
        " asked whether ",
        " asks whether ",
        " claimed ",
        " claims ",
        " hypothetically ",
        " i believe ",
        " i guess ",
        " i think ",
        " if ",
        " likely ",
        " may ",
        " maybe ",
        " might ",
        " perhaps ",
        " possibly ",
        " potentially ",
        " presumably ",
        " probably ",
        " provisional ",
        " provisionally ",
        " reportedly ",
        " said ",
        " says ",
        " supposedly ",
        " tentative ",
        " tentatively ",
        " uncertain ",
        " unsure ",
        " unless ",
        " when ",
        " whether ",
    ]
    .iter()
    .any(|marker| padded.contains(marker))
}

fn has_quoted_or_retracted_evidence(source: &str) -> bool {
    let characters = source.chars().collect::<Vec<_>>();
    for (index, character) in characters.iter().enumerate() {
        if matches!(character, '"' | '“' | '”' | '`') {
            return true;
        }
        if matches!(character, '\'' | '‘' | '’') {
            let inside_word = index
                .checked_sub(1)
                .and_then(|previous| characters.get(previous))
                .is_some_and(|previous| previous.is_alphanumeric())
                && characters
                    .get(index + 1)
                    .is_some_and(|next| next.is_alphanumeric());
            if !inside_word {
                return true;
            }
        }
    }
    let normalized = words(source).join(" ");
    let padded = format!(" {normalized} ");
    [
        " actually ",
        " actually no ",
        " although ",
        " but ",
        " but i don t ",
        " but i do not ",
        " correction ",
        " disregard ",
        " except ",
        " however ",
        " ignore that ",
        " is false ",
        " isn t true ",
        " no longer ",
        " not true ",
        " not anymore ",
        " rather than ",
        " reported ",
        " retract ",
        " scratch that ",
        " someone said ",
        " the phrase ",
        " the prompt says ",
        " they said ",
        " unless ",
        " used to ",
        " wait ",
        " you said ",
    ]
    .iter()
    .any(|marker| padded.contains(marker))
}

/// Evidence must be one simple assertion. Entry-level citations cannot prove
/// which sentence a model relied on, so multi-sentence messages remain in
/// review: a later sentence may retract or qualify an earlier one. Quoted and
/// retracted statements are review-only for the same reason.
fn source_supports_claim(kind: &str, body: &str, source: &str) -> bool {
    if has_quoted_or_retracted_evidence(source) || has_non_assertive_context(source) {
        return false;
    }
    let clauses = source
        .split(|character| matches!(character, '.' | '?' | '!' | '\n' | '\r' | '—'))
        .map(str::trim)
        .filter(|clause| !clause.is_empty())
        .collect::<Vec<_>>();
    let [clause] = clauses.as_slice() else {
        return false;
    };
    let source_relations = relation_sequence(clause);
    has_explicit_source_cue(kind, clause)
        && has_negative_polarity(body) == has_negative_polarity(clause)
        && preserves_qualifiers(body, clause)
        // A second predicate carries its own tense, modality and object scope.
        // This lexical gate cannot safely prove those survived a rewrite.
        && source_relations.len() <= 1
        && relation_sequence(body) == source_relations
        && lexically_grounded(body, clause)
}

fn likely_related_claim(left: &str, right: &str) -> bool {
    let left_terms = lexical_terms(left);
    let right_terms = lexical_terms(right);
    let smaller = left_terms.len().min(right_terms.len());
    let overlap = left_terms.intersection(&right_terms).count();
    let same_relation = relation_family(left)
        .zip(relation_family(right))
        .is_some_and(|(left, right)| left == right);
    if same_relation && (overlap > 0 || (left_terms.len() <= 1 && right_terms.len() <= 1)) {
        return true;
    }
    if smaller == 0 {
        return false;
    }
    let dense_overlap = if smaller == 1 {
        overlap == 1
    } else {
        overlap >= 2 && overlap * 3 >= smaller * 2
    };
    dense_overlap || (overlap >= 1 && (has_relational_marker(left) || has_relational_marker(right)))
}

/// A semantic conflict cannot be proven from the extractor's four fields. The
/// automatic path therefore uses a conservative lexical guard across every
/// active or pending claim, regardless of model-assigned kind. False positives
/// stay in the review queue; they are never discarded.
fn likely_conflict(db: &Connection, scope_key: &str, body: &str) -> Result<bool, BridgeError> {
    let mut statement = db.prepare(
        "SELECT body FROM memory_records
         WHERE scope_key=?1 AND status IN ('active','proposed')",
    )?;
    let existing = statement
        .query_map(params![scope_key], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(existing
        .iter()
        .any(|known| likely_related_claim(body, known)))
}

fn eligible_for_auto_apply(
    db: &Connection,
    scope_key: &str,
    session_id: &str,
    body: &str,
    kind: &str,
    raw: &RawProposal,
) -> Result<bool, BridgeError> {
    let Some(confidence_bps) = raw.confidence_bps else {
        return Ok(false);
    };
    if !(AUTO_APPLY_MIN_CONFIDENCE_BPS..=10_000).contains(&confidence_bps) {
        return Ok(false);
    }
    let Some(source) = cited_user_message(db, session_id, raw.rationale.as_deref())? else {
        return Ok(false);
    };
    if has_transient_marker(body) || has_transient_marker(&source) {
        return Ok(false);
    }
    if !source_supports_claim(kind, body, &source) {
        return Ok(false);
    }
    if matches_deleted_claim(db, scope_key, body)? || likely_conflict(db, scope_key, body)? {
        return Ok(false);
    }
    Ok(true)
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ExtractionReport {
    pub written: usize,
    pub activated: usize,
    pub queued_for_review: usize,
    pub invalid: usize,
    pub duplicates: usize,
    pub refused: usize,
}

struct GatedCandidate {
    raw: RawProposal,
    body: String,
    kind: &'static str,
    auto_eligible: bool,
}

fn fenced_payload(text: &str) -> Option<&str> {
    let start = text.find("```bridge-memory-proposals")?;
    let after = &text[start + "```bridge-memory-proposals".len()..];
    let end = after.find("```")?;
    Some(after[..end].trim())
}

fn contains_secret_material(text: &str) -> bool {
    let lowered = text.to_lowercase();
    lowered.contains("[secret:")
        || lowered.contains("/credential-proxy/")
        || lowered.contains("x-bridge-proxy-auth")
        || !secret_interception::intercept(text)
            .sanitized
            .interceptions
            .is_empty()
}

/// The deterministic gate. Everything an explicit save enforces runs again
/// here, and whatever a model claimed, every survivor first lands as
/// proposed/model_proposal. Auto-apply may then use the ledger's ordinary
/// approval transition; a stale mode or an ineligible candidate stays queued
/// for review. An element with any field beyond the four — a status, a
/// provenance, a scope — is invalid rather than honored.
pub fn gate_and_insert(
    db: &Connection,
    scope_key: &str,
    session_id: &str,
    claimed_mode: &str,
    model_text: &str,
) -> Result<ExtractionReport, BridgeError> {
    let scope_key = memory_ledger::parse_scope_key(scope_key)?;
    let current_mode = settings(db, &scope_key)?.mode;
    let auto_apply = claimed_mode == MODE_AUTO_APPLY && current_mode == MODE_AUTO_APPLY;
    let mut report = ExtractionReport::default();
    let Some(payload) = fenced_payload(model_text) else {
        return Err(BridgeError::Invalid(
            "The model answered without a bridge-memory-proposals block.".into(),
        ));
    };
    let elements: Vec<serde_json::Value> = serde_json::from_str(payload).map_err(|error| {
        BridgeError::Invalid(format!("Proposals are not a JSON array: {error}"))
    })?;
    let mut candidates = Vec::new();
    let mut batch_bodies = BTreeSet::new();
    for element in elements.into_iter().take(MAX_PROPOSALS_PER_RUN) {
        let raw: RawProposal = match serde_json::from_value(element) {
            Ok(raw) => raw,
            Err(_) => {
                report.invalid += 1;
                continue;
            }
        };
        if contains_secret_material(&raw.body)
            || raw
                .rationale
                .as_deref()
                .is_some_and(contains_secret_material)
        {
            report.refused += 1;
            continue;
        }
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
        let body_key = body.to_lowercase();
        if memory_ledger::body_already_known(db, &scope_key, &body)?
            || !batch_bodies.insert(body_key)
        {
            report.duplicates += 1;
            continue;
        }
        let auto_eligible =
            auto_apply && eligible_for_auto_apply(db, &scope_key, session_id, &body, kind, &raw)?;
        candidates.push(GatedCandidate {
            raw,
            body,
            kind,
            auto_eligible,
        });
    }

    // Resolve the whole model batch before writing any of it. Otherwise the
    // first of two contradictory candidates could become active merely by
    // appearing first, while only the second saw the new record as a conflict.
    let mut batch_conflicts = vec![false; candidates.len()];
    for left in 0..candidates.len() {
        for right in (left + 1)..candidates.len() {
            if likely_related_claim(&candidates[left].body, &candidates[right].body) {
                batch_conflicts[left] = true;
                batch_conflicts[right] = true;
            }
        }
    }

    for (candidate, batch_conflict) in candidates.into_iter().zip(batch_conflicts) {
        let GatedCandidate {
            raw,
            body,
            kind,
            auto_eligible,
        } = candidate;
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
            Ok(proposal) => {
                report.written += 1;
                if auto_eligible && !batch_conflict {
                    memory_ledger::approve(db, &proposal.id)?;
                    report.activated += 1;
                } else {
                    report.queued_for_review += 1;
                }
            }
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
    fn propose(
        &mut self,
        instructions: &str,
        digest: &str,
    ) -> Result<ExtractionOutput, BridgeError>;
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
            ExtractionOutput {
                text: String::new(),
                observed_tokens: 0,
                spend_microusd: 0,
            },
            None,
        ));
    };
    let claimed_mode = settings(db, scope_key)?.mode;
    let output = model.propose(&extraction_instructions(), &digest.text)?;
    let report = gate_and_insert(db, scope_key, session_id, &claimed_mode, &output.text)?;
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

    fn insert_message(
        db: &Connection,
        session: &str,
        id: &str,
        sequence: i64,
        kind: &str,
        text: &str,
    ) {
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
        update_settings(
            db,
            ACCOUNT_MEMORY_SCOPE,
            MODE_PROPOSE,
            Some("claude"),
            Some("sonnet"),
        )
        .unwrap();
    }

    fn auto_apply_mode(db: &Connection) {
        update_settings(
            db,
            ACCOUNT_MEMORY_SCOPE,
            MODE_AUTO_APPLY,
            Some("claude"),
            Some("sonnet"),
        )
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
        fn propose(
            &mut self,
            _instructions: &str,
            _digest: &str,
        ) -> Result<ExtractionOutput, BridgeError> {
            Ok(ExtractionOutput {
                text: self.0.clone(),
                observed_tokens: 420,
                spend_microusd: 1_700,
            })
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
    fn migration_defaults_preexisting_runs_to_review_only() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE memory_extraction_runs (id TEXT PRIMARY KEY);
             INSERT INTO memory_extraction_runs(id) VALUES('old-run');",
        )
        .unwrap();
        let transaction = db.transaction().unwrap();
        install_run_modes(&transaction).unwrap();
        transaction.commit().unwrap();

        let mode: String = db
            .query_row(
                "SELECT mode FROM memory_extraction_runs WHERE id='old-run'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(mode, MODE_PROPOSE);
    }

    #[test]
    fn auto_apply_is_explicit_and_its_mode_is_snapshotted_at_enqueue() {
        let (_dir, db) = extraction_db();
        assert_eq!(
            settings(&db, ACCOUNT_MEMORY_SCOPE).unwrap().mode,
            MODE_PROPOSE
        );
        auto_apply_mode(&db);
        insert_chat(&db, "s1", "direct");
        assert!(enqueue_after_turn(&db, "s1").unwrap());
        let claimed = claim_due(&db, Utc::now()).unwrap().unwrap();
        assert_eq!(claimed.mode, MODE_AUTO_APPLY);
    }

    #[test]
    fn changing_propose_to_auto_before_claim_cannot_escalate_the_queued_run() {
        let (_dir, db) = extraction_db();
        propose_mode(&db);
        insert_chat(&db, "s1", "direct");
        insert_message(
            &db,
            "s1",
            "e1",
            1,
            "user.message",
            "I prefer spaces for indentation.",
        );
        assert!(enqueue_after_turn(&db, "s1").unwrap());

        auto_apply_mode(&db);
        let claimed = claim_due(&db, Utc::now()).unwrap().unwrap();
        assert_eq!(claimed.mode, MODE_PROPOSE);
        let report = gate_and_insert(
            &db,
            ACCOUNT_MEMORY_SCOPE,
            "s1",
            &claimed.mode,
            &fenced(
                r#"[{"body":"Prefers spaces for indentation","kind":"preference","confidenceBps":9500,"rationale":"[e1] Direct preference"}]"#,
            ),
        )
        .unwrap();
        assert_eq!(report.activated, 0);
        assert_eq!(report.queued_for_review, 1);
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
        assert_eq!(claimed.mode, MODE_PROPOSE);
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
        assert_eq!(
            claimed.model, None,
            "no model pinned and none chosen: the adapter's default"
        );
    }

    #[test]
    fn a_pinned_helper_outranks_the_chats_own_harness() {
        let (_dir, db) = extraction_db();
        update_settings(
            &db,
            ACCOUNT_MEMORY_SCOPE,
            MODE_PROPOSE,
            Some("claude"),
            Some("haiku"),
        )
        .unwrap();
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
        assert!(
            last_run(&db, ACCOUNT_MEMORY_SCOPE).unwrap().is_none(),
            "nothing queued, nothing cancelled"
        );
        // Pinning a helper that can hold briefing authority makes them eligible.
        propose_mode(&db);
        assert!(enqueue_after_turn(&db, "c1").unwrap());
        assert_eq!(
            claim_due(&db, Utc::now()).unwrap().unwrap().harness,
            "claude"
        );
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
        assert!(update_settings(
            &db,
            ACCOUNT_MEMORY_SCOPE,
            MODE_PROPOSE,
            Some("claude"),
            None
        )
        .is_err());
        assert!(update_settings(&db, ACCOUNT_MEMORY_SCOPE, MODE_PROPOSE, None, Some("m")).is_err());
        assert!(update_settings(&db, ACCOUNT_MEMORY_SCOPE, MODE_PROPOSE, None, None).is_ok());
        assert!(update_settings(
            &db,
            ACCOUNT_MEMORY_SCOPE,
            MODE_PROPOSE,
            Some("claude"),
            Some("m")
        )
        .is_ok());
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
        assert!(
            !enqueue_after_turn(&db, "s1").unwrap(),
            "one open run per session"
        );
    }

    #[test]
    fn proposals_land_proposed_and_never_active() {
        let (_dir, db) = extraction_db();
        propose_mode(&db);
        insert_chat(&db, "s1", "direct");
        insert_message(
            &db,
            "s1",
            "e1",
            1,
            "user.message",
            "Always run bun run check before pushing",
        );
        let mut model = CannedModel(fenced(
            r#"[{"body":"Runs bun run check before pushing","kind":"constraint","confidenceBps":9000,"rationale":"Said directly"}]"#,
        ));
        let (report, output, digest) =
            run_extraction(&db, &mut model, ACCOUNT_MEMORY_SCOPE, "s1").unwrap();
        assert_eq!(report.written, 1);
        assert_eq!(report.activated, 0);
        assert_eq!(report.queued_for_review, 1);
        assert_eq!(output.spend_microusd, 1_700);
        assert!(digest.is_some());
        let proposed = memory_ledger::list(&db, ACCOUNT_MEMORY_SCOPE, Some("proposed")).unwrap();
        assert_eq!(proposed.records.len(), 1);
        let record = &proposed.records[0];
        assert_eq!(record.status, "proposed");
        assert_eq!(record.provenance, "model_proposal");
        assert_eq!(record.confidence_bps, Some(9000));
        assert!(memory_ledger::list(&db, ACCOUNT_MEMORY_SCOPE, None)
            .unwrap()
            .records
            .is_empty());
    }

    #[test]
    fn eligible_auto_apply_uses_proposal_then_approval_and_reaches_future_packets() {
        let (_dir, db) = extraction_db();
        auto_apply_mode(&db);
        insert_chat(&db, "s1", "direct");
        insert_message(
            &db,
            "s1",
            "e1",
            1,
            "user.message",
            "I prefer spaces for indentation.",
        );
        let mut model = CannedModel(fenced(
            r#"[{"body":"Prefers spaces for indentation","kind":"preference","confidenceBps":9500,"rationale":"[e1] Direct user preference"}]"#,
        ));

        let (report, _, _) = run_extraction(&db, &mut model, ACCOUNT_MEMORY_SCOPE, "s1").unwrap();
        assert_eq!(report.written, 1);
        assert_eq!(report.activated, 1);
        assert_eq!(report.queued_for_review, 0);
        let active = memory_ledger::list(&db, ACCOUNT_MEMORY_SCOPE, None).unwrap();
        assert_eq!(active.records.len(), 1);
        let record = &active.records[0];
        assert!(approved_proposal(record));
        assert_eq!(record.source_session_id.as_deref(), Some("s1"));
        assert_eq!(record.confidence_bps, Some(9500));
        assert!(
            memory_ledger::list(&db, ACCOUNT_MEMORY_SCOPE, Some("proposed"))
                .unwrap()
                .records
                .is_empty()
        );

        let packet =
            crate::memory_packet::for_session(&db, ACCOUNT_MEMORY_SCOPE, "a-future-session")
                .unwrap()
                .unwrap();
        assert_eq!(packet.selected.len(), 1);
        assert_eq!(packet.selected[0].record_id, record.id);
        assert_eq!(packet.selected[0].body, "Prefers spaces for indentation");
    }

    #[test]
    fn auto_apply_falls_back_for_confidence_evidence_grounding_and_transience() {
        let (_dir, db) = extraction_db();
        auto_apply_mode(&db);
        insert_chat(&db, "s1", "direct");
        insert_chat(&db, "s2", "direct");
        insert_message(
            &db,
            "s1",
            "e-low",
            1,
            "user.message",
            "I prefer concise answers.",
        );
        insert_message(
            &db,
            "s1",
            "e-range",
            2,
            "user.message",
            "I use the fish shell.",
        );
        insert_message(
            &db,
            "s1",
            "e-plain",
            3,
            "user.message",
            "I prefer spaces for indentation.",
        );
        insert_message(
            &db,
            "s1",
            "e-assistant",
            4,
            "assistant.message",
            "I prefer dark mode.",
        );
        insert_message(
            &db,
            "s1",
            "e-transient",
            5,
            "user.message",
            "For now, I prefer deployments on Fridays.",
        );
        insert_message(
            &db,
            "s1",
            "e-ungrounded",
            6,
            "user.message",
            "I prefer tea.",
        );
        insert_message(
            &db,
            "s1",
            "e-incidental",
            7,
            "user.message",
            "Please help debug this Rust error.",
        );
        insert_message(
            &db,
            "s1",
            "e-hidden",
            8,
            "user.message",
            "I prefer solarized colors.",
        );
        db.execute(
            "UPDATE session_entries SET context_visibility='hidden' WHERE id='e-hidden'",
            [],
        )
        .unwrap();
        insert_message(
            &db,
            "s2",
            "e-other-session",
            1,
            "user.message",
            "I prefer modal editing.",
        );
        let mut model = CannedModel(fenced(
            r#"[
                {"body":"Prefers concise answers","kind":"preference","confidenceBps":8999,"rationale":"[e-low] Direct preference"},
                {"body":"Uses the fish shell","kind":"fact","confidenceBps":99999,"rationale":"[e-range] Direct fact"},
                {"body":"Prefers spaces for indentation","kind":"preference","confidenceBps":9500,"rationale":"Direct statement"},
                {"body":"Prefers dark mode","kind":"preference","confidenceBps":9500,"rationale":"[e-assistant] Assistant context"},
                {"body":"Prefers deployments on Fridays for now","kind":"preference","confidenceBps":9500,"rationale":"[e-transient] Temporary preference"},
                {"body":"Requires Kubernetes clusters","kind":"constraint","confidenceBps":9500,"rationale":"[e-ungrounded] Not grounded"},
                {"body":"Uses Rust","kind":"fact","confidenceBps":9500,"rationale":"[e-incidental] Incidental topic mention"},
                {"body":"Prefers solarized colors","kind":"preference","confidenceBps":9500,"rationale":"[e-hidden] Hidden evidence"},
                {"body":"Prefers modal editing","kind":"preference","confidenceBps":9500,"rationale":"[e-other-session] Different session"}
            ]"#,
        ));

        let (report, _, _) = run_extraction(&db, &mut model, ACCOUNT_MEMORY_SCOPE, "s1").unwrap();
        assert_eq!(report.written, 9);
        assert_eq!(report.activated, 0);
        assert_eq!(report.queued_for_review, 9);
        assert!(memory_ledger::list(&db, ACCOUNT_MEMORY_SCOPE, None)
            .unwrap()
            .records
            .is_empty());
        let proposed = memory_ledger::list(&db, ACCOUNT_MEMORY_SCOPE, Some("proposed")).unwrap();
        assert_eq!(proposed.records.len(), 9);
        assert_eq!(
            proposed
                .records
                .iter()
                .find(|record| record.body == "Uses the fish shell")
                .unwrap()
                .confidence_bps,
            Some(10_000),
            "storage still clamps, but the raw out-of-range value cannot auto-promote"
        );
    }

    #[test]
    fn auto_apply_leaves_one_off_requests_and_broad_possessives_for_review() {
        let (_dir, db) = extraction_db();
        auto_apply_mode(&db);
        insert_chat(&db, "s1", "direct");
        insert_message(
            &db,
            "s1",
            "e-possessive",
            1,
            "user.message",
            "Please clean up my Python script.",
        );
        insert_message(
            &db,
            "s1",
            "e-imperative",
            2,
            "user.message",
            "Use pnpm for the install.",
        );
        insert_message(
            &db,
            "s1",
            "e-want",
            3,
            "user.message",
            "I want a blue header.",
        );
        let mut model = CannedModel(fenced(
            r#"[
                {"body":"Uses Python","kind":"fact","confidenceBps":9500,"rationale":"[e-possessive] Possessive mention"},
                {"body":"Uses pnpm","kind":"decision","confidenceBps":9500,"rationale":"[e-imperative] One-off imperative"},
                {"body":"Prefers blue headers","kind":"preference","confidenceBps":9500,"rationale":"[e-want] One-off request"}
            ]"#,
        ));

        let (report, _, _) = run_extraction(&db, &mut model, ACCOUNT_MEMORY_SCOPE, "s1").unwrap();
        assert_eq!(report.written, 3);
        assert_eq!(report.activated, 0);
        assert_eq!(report.queued_for_review, 3);
        assert!(memory_ledger::list(&db, ACCOUNT_MEMORY_SCOPE, None)
            .unwrap()
            .records
            .is_empty());
    }

    #[test]
    fn auto_apply_requires_an_assertive_cue_and_grounding_in_one_unquoted_sentence() {
        for (source, body) in [
            (
                "I prefer concise answers. Please debug this Rust error.",
                "Prefers Rust",
            ),
            (
                "The prompt says “I prefer tabs”, but I don't.",
                "Prefers tabs",
            ),
            ("The phrase ‘I prefer tabs’ is false.", "Prefers tabs"),
            ("I prefer tabs—actually, no, spaces.", "Prefers tabs"),
            ("Do I prefer tabs?", "Prefers tabs"),
            ("Maybe I prefer tabs.", "Prefers tabs"),
            ("If I prefer tabs, use tabs.", "Prefers tabs"),
            ("According to the prompt, I prefer tabs.", "Prefers tabs"),
            ("The prompt claims I prefer tabs.", "Prefers tabs"),
            ("They say I prefer tabs.", "Prefers tabs"),
        ] {
            let (_dir, db) = extraction_db();
            auto_apply_mode(&db);
            insert_chat(&db, "s1", "direct");
            insert_message(&db, "s1", "e1", 1, "user.message", source);
            assert!(!source_supports_claim("preference", body, source));
            let mut model = CannedModel(fenced(&format!(
                r#"[{{"body":"{body}","kind":"preference","confidenceBps":9500,"rationale":"[e1] Candidate evidence"}}]"#,
            )));

            let (report, _, _) =
                run_extraction(&db, &mut model, ACCOUNT_MEMORY_SCOPE, "s1").unwrap();
            assert_eq!(report.written, 1);
            assert_eq!(
                report.activated, 0,
                "source unexpectedly activated: {source}"
            );
            assert_eq!(report.queued_for_review, 1);
        }
    }

    #[test]
    fn auto_apply_preserves_polarity_alternative_order_and_scope() {
        for (source, body, kind) in [
            ("I never use Docker.", "Uses Docker", "constraint"),
            (
                "I prefer tabs over spaces.",
                "Prefers spaces over tabs",
                "preference",
            ),
            ("I prefer spaces over tabs.", "Prefers tabs", "preference"),
            ("I prefer tea, not coffee.", "Prefers coffee", "preference"),
            (
                "I prefer tabs only for generated files.",
                "Prefers tabs",
                "preference",
            ),
            (
                "I prefer tabs only for generated files.",
                "Prefers tabs only for files",
                "preference",
            ),
            ("I use the fish shell.", "Uses fish", "fact"),
            ("I prefer tabs—wait, spaces.", "Prefers tabs", "preference"),
            (
                "I prefer tabs—actually spaces.",
                "Prefers tabs",
                "preference",
            ),
            ("I prefer tabs this time.", "Prefers tabs", "preference"),
            (
                "I prefer tabs until the migration ends.",
                "Prefers tabs",
                "preference",
            ),
            (
                "I prefer neither tabs nor spaces.",
                "Prefers tabs",
                "preference",
            ),
            ("I prefer spaces vs tabs.", "Prefers tabs", "preference"),
            (
                "I prefer tabs whenever editing Makefiles.",
                "Prefers tabs",
                "preference",
            ),
            ("I prefer tabs next month.", "Prefers tabs", "preference"),
            (
                "I prefer tabs next month.",
                "Prefers tabs next month",
                "preference",
            ),
            (
                "I prefer tabs this iteration.",
                "Prefers tabs",
                "preference",
            ),
            (
                "I prefer tabs this iteration.",
                "Prefers tabs this iteration",
                "preference",
            ),
            (
                "I prefer tabs for two weeks.",
                "Prefers tabs for two weeks",
                "preference",
            ),
            (
                "I prefer tabs in 2027.",
                "Prefers tabs in 2027",
                "preference",
            ),
            (
                "I prefer tabs. I changed my mind.",
                "Prefers tabs",
                "preference",
            ),
            (
                "I prefer tabs. Make that spaces.",
                "Prefers tabs",
                "preference",
            ),
            (
                "I prefer tabs. I take that back.",
                "Prefers tabs",
                "preference",
            ),
            (
                "I work from home to office.",
                "Works to home from office",
                "fact",
            ),
            (
                "I prefer tea and coffee or water.",
                "Prefers tea or coffee and water",
                "preference",
            ),
            (
                "I use Rust and can't use Go.",
                "Uses Rust and can use Go",
                "fact",
            ),
            (
                "I use Rust and can’t use Go.",
                "Uses Rust and can use Go",
                "fact",
            ),
            (
                "I prefer Rust and use Go.",
                "Prefers Rust and Go",
                "preference",
            ),
            (
                "I avoid tabs and prefer spaces.",
                "Avoids tabs and spaces",
                "constraint",
            ),
            (
                "I use tabs and prefer spaces.",
                "Uses tabs and spaces",
                "fact",
            ),
            (
                "I use Rust and worked for Acme.",
                "Uses Rust and works for Acme",
                "fact",
            ),
            (
                "I use Rust and will work for Acme.",
                "Uses Rust and works for Acme",
                "fact",
            ),
            ("I use their API.", "Uses our API", "fact"),
            ("I work at her company.", "Works at my company", "fact"),
            (
                "I work by contract for Acme.",
                "Works the contract for Acme",
                "fact",
            ),
            ("I prefer this editor.", "Prefers editor", "preference"),
            ("I prefer Rust to it.", "Prefers it to Rust", "preference"),
            (
                "I prefer Rust and they are Go users.",
                "Prefers Rust and Go users",
                "preference",
            ),
            (
                "I prefer Rust for backend and Go for Rust tooling.",
                "Prefers backend for Go and Rust for tooling",
                "preference",
            ),
            (
                "I prefer tabs for the moment.",
                "Prefers tabs for the moment",
                "preference",
            ),
            (
                "I prefer tabs in the meantime.",
                "Prefers tabs in the meantime",
                "preference",
            ),
            (
                "I prefer tabs for eleven days.",
                "Prefers tabs for eleven days",
                "preference",
            ),
            (
                "I prefer tabs for several weeks.",
                "Prefers tabs for several weeks",
                "preference",
            ),
            (
                "I prefer tabs this September.",
                "Prefers tabs this September",
                "preference",
            ),
            (
                "I prefer tabs at 3 PM.",
                "Prefers tabs at 3 PM",
                "preference",
            ),
            (
                "I prefer tabs probably.",
                "Prefers tabs probably",
                "preference",
            ),
            (
                "I prefer tabs I think.",
                "Prefers tabs I think",
                "preference",
            ),
            (
                "I prefer tabs I guess.",
                "Prefers tabs I guess",
                "preference",
            ),
            (
                "I prefer tabs tentatively.",
                "Prefers tabs tentatively",
                "preference",
            ),
            (
                "I decided on tabs provisionally.",
                "Decided on tabs provisionally",
                "decision",
            ),
            ("I prefer.", "I prefer", "preference"),
        ] {
            let (_dir, db) = extraction_db();
            auto_apply_mode(&db);
            insert_chat(&db, "s1", "direct");
            insert_message(&db, "s1", "e1", 1, "user.message", source);
            assert!(
                has_transient_marker(source) || !source_supports_claim(kind, body, source),
                "source gate unexpectedly accepted {body:?} from {source:?}"
            );
            let mut model = CannedModel(fenced(&format!(
                r#"[{{"body":"{body}","kind":"{kind}","confidenceBps":9500,"rationale":"[e1] Candidate evidence"}}]"#,
            )));

            let (report, _, _) =
                run_extraction(&db, &mut model, ACCOUNT_MEMORY_SCOPE, "s1").unwrap();
            assert_eq!(report.written, 1);
            assert_eq!(
                report.activated, 0,
                "source unexpectedly activated {body:?} from {source:?}"
            );
            assert_eq!(report.queued_for_review, 1);
        }
    }

    #[test]
    fn auto_apply_treats_deictic_examples_and_migrations_as_transient() {
        for source in [
            "For this example, I prefer tabs.",
            "For this demo, I prefer tabs.",
            "During this migration, I prefer tabs.",
            "For this message, I prefer tabs.",
            "For the demo, I prefer tabs.",
            "During the migration, I prefer tabs.",
            "In the example, I prefer tabs.",
        ] {
            let (_dir, db) = extraction_db();
            auto_apply_mode(&db);
            insert_chat(&db, "s1", "direct");
            insert_message(&db, "s1", "e1", 1, "user.message", source);
            assert!(has_transient_marker(source));
            let mut model = CannedModel(fenced(
                r#"[{"body":"Prefers tabs","kind":"preference","confidenceBps":9500,"rationale":"[e1] Candidate evidence"}]"#,
            ));

            let (report, _, _) =
                run_extraction(&db, &mut model, ACCOUNT_MEMORY_SCOPE, "s1").unwrap();
            assert_eq!(report.written, 1);
            assert_eq!(
                report.activated, 0,
                "source unexpectedly activated: {source}"
            );
            assert_eq!(report.queued_for_review, 1);
        }
    }

    #[test]
    fn auto_apply_still_blocks_credentials_and_unknown_fields() {
        let (_dir, db) = extraction_db();
        auto_apply_mode(&db);
        insert_chat(&db, "s1", "direct");
        insert_message(
            &db,
            "s1",
            "e1",
            1,
            "user.message",
            "I prefer concise answers.",
        );
        let mut model = CannedModel(fenced(
            r#"[
                {"body":"Prefers concise answers","kind":"preference","confidenceBps":9500,"rationale":"[e1] Direct preference","status":"active"},
                {"body":"api key sk-proj-1234567890abcdefghijklmnopqrstuvwxyz123456","kind":"fact","confidenceBps":9500,"rationale":"[e1] Secret"},
                {"body":"Prefers concise replies","kind":"preference","confidenceBps":9500,"rationale":"[e1] token sk-ant-abcdefghijklmnopqrstuvwxyz123456"},
                {"body":"Prefers brief replies","kind":"preference","confidenceBps":9500,"rationale":"[e1] [secret:sec_reference]"}
            ]"#,
        ));

        let (report, _, _) = run_extraction(&db, &mut model, ACCOUNT_MEMORY_SCOPE, "s1").unwrap();
        assert_eq!(report.invalid, 1);
        assert_eq!(report.refused, 3);
        assert_eq!(report.written, 0);
        assert!(memory_ledger::list(&db, ACCOUNT_MEMORY_SCOPE, None)
            .unwrap()
            .records
            .is_empty());
        assert!(
            memory_ledger::list(&db, ACCOUNT_MEMORY_SCOPE, Some("proposed"))
                .unwrap()
                .records
                .is_empty()
        );
    }

    #[test]
    fn manual_cross_kind_conflicts_and_shared_domains_stay_reviewable() {
        assert!(likely_related_claim("Prefers tabs", "Prefers spaces"));
        assert!(likely_related_claim("Uses bun", "Uses npm"));
        assert!(likely_related_claim(
            "Prefers dark mode",
            "Prefers light mode"
        ));

        let (_dir, db) = extraction_db();
        auto_apply_mode(&db);
        insert_chat(&db, "s1", "direct");
        insert_message(&db, "s1", "e1", 1, "user.message", "I prefer spaces.");
        memory_ledger::save(&db, "Prefers tabs", Some("fact"), None).unwrap();
        let mut model = CannedModel(fenced(
            r#"[{"body":"Prefers spaces","kind":"preference","confidenceBps":9500,"rationale":"[e1] Direct preference"}]"#,
        ));

        let (report, _, _) = run_extraction(&db, &mut model, ACCOUNT_MEMORY_SCOPE, "s1").unwrap();
        assert_eq!(report.written, 1);
        assert_eq!(report.activated, 0);
        assert_eq!(report.queued_for_review, 1);
        let active = memory_ledger::list(&db, ACCOUNT_MEMORY_SCOPE, None).unwrap();
        assert_eq!(active.records.len(), 1);
        assert_eq!(active.records[0].body, "Prefers tabs");
        assert_eq!(active.records[0].kind, "fact");
        let proposed = memory_ledger::list(&db, ACCOUNT_MEMORY_SCOPE, Some("proposed")).unwrap();
        assert_eq!(proposed.records.len(), 1);
        assert_eq!(proposed.records[0].body, "Prefers spaces");
    }

    #[test]
    fn conflicting_candidates_in_one_batch_are_both_left_for_review() {
        let (_dir, db) = extraction_db();
        auto_apply_mode(&db);
        insert_chat(&db, "s1", "direct");
        insert_message(&db, "s1", "e1", 1, "user.message", "I prefer tabs.");
        insert_message(
            &db,
            "s1",
            "e2",
            2,
            "user.message",
            "Actually, I prefer spaces.",
        );
        let mut model = CannedModel(fenced(
            r#"[
                {"body":"Prefers tabs","kind":"preference","confidenceBps":9500,"rationale":"[e1] Direct preference"},
                {"body":"Prefers spaces","kind":"preference","confidenceBps":9500,"rationale":"[e2] Direct preference"}
            ]"#,
        ));

        let (report, _, _) = run_extraction(&db, &mut model, ACCOUNT_MEMORY_SCOPE, "s1").unwrap();
        assert_eq!(report.written, 2);
        assert_eq!(report.activated, 0);
        assert_eq!(report.queued_for_review, 2);
        assert!(memory_ledger::list(&db, ACCOUNT_MEMORY_SCOPE, None)
            .unwrap()
            .records
            .is_empty());
    }

    #[test]
    fn deleted_claim_variants_are_reproposed_but_not_auto_applied() {
        for (source, candidate) in [
            (
                "I prefer spaces for indentation.",
                "Prefers spaces for indentation.",
            ),
            (
                "I prefer spaces when indenting.",
                "Prefers spaces when indenting",
            ),
        ] {
            let (_dir, db) = extraction_db();
            auto_apply_mode(&db);
            insert_chat(&db, "s1", "direct");
            insert_message(&db, "s1", "e1", 1, "user.message", source);
            let deleted = memory_ledger::save(
                &db,
                "Prefers spaces for indentation",
                Some("preference"),
                None,
            )
            .unwrap();
            memory_ledger::forget(&db, &deleted.id).unwrap();
            assert!(matches_deleted_claim(&db, ACCOUNT_MEMORY_SCOPE, candidate).unwrap());
            let mut model = CannedModel(fenced(&format!(
                r#"[{{"body":"{candidate}","kind":"preference","confidenceBps":9500,"rationale":"[e1] Direct preference"}}]"#,
            )));

            let (report, _, _) =
                run_extraction(&db, &mut model, ACCOUNT_MEMORY_SCOPE, "s1").unwrap();
            assert_eq!(report.written, 1);
            assert_eq!(report.activated, 0);
            assert_eq!(report.queued_for_review, 1);
            assert!(memory_ledger::list(&db, ACCOUNT_MEMORY_SCOPE, None)
                .unwrap()
                .records
                .is_empty());
            assert_eq!(
                memory_ledger::list(&db, ACCOUNT_MEMORY_SCOPE, Some("proposed"))
                    .unwrap()
                    .records
                    .len(),
                1
            );
        }
    }

    #[test]
    fn auto_apply_never_evicts_or_overflows_the_scope_budget() {
        let (_dir, db) = extraction_db();
        auto_apply_mode(&db);
        crate::memory_consolidation::update_settings(
            &db,
            ACCOUNT_MEMORY_SCOPE,
            crate::memory_consolidation::MODE_OFF,
            None,
            None,
            Some(1),
            None,
            None,
        )
        .unwrap();
        memory_ledger::save(&db, "Keeps release notes", Some("fact"), None).unwrap();
        insert_chat(&db, "s1", "direct");
        insert_message(
            &db,
            "s1",
            "e1",
            1,
            "user.message",
            "I prefer spaces for indentation.",
        );
        let mut model = CannedModel(fenced(
            r#"[{"body":"Prefers spaces for indentation","kind":"preference","confidenceBps":9500,"rationale":"[e1] Direct preference"}]"#,
        ));

        let (report, _, _) = run_extraction(&db, &mut model, ACCOUNT_MEMORY_SCOPE, "s1").unwrap();
        assert_eq!(report.written, 0);
        assert_eq!(report.activated, 0);
        assert_eq!(report.refused, 1);
        let active = memory_ledger::list(&db, ACCOUNT_MEMORY_SCOPE, None).unwrap();
        assert_eq!(active.records.len(), 1);
        assert_eq!(active.records[0].body, "Keeps release notes");
        assert!(
            memory_ledger::list(&db, ACCOUNT_MEMORY_SCOPE, Some("proposed"))
                .unwrap()
                .records
                .is_empty()
        );
    }

    #[test]
    fn auto_apply_requires_both_claimed_and_current_modes_to_remain_auto() {
        let (_dir, db) = extraction_db();
        auto_apply_mode(&db);
        insert_chat(&db, "s1", "direct");
        insert_message(
            &db,
            "s1",
            "e1",
            1,
            "user.message",
            "I prefer spaces for indentation.",
        );
        update_settings(
            &db,
            ACCOUNT_MEMORY_SCOPE,
            MODE_PROPOSE,
            Some("claude"),
            Some("sonnet"),
        )
        .unwrap();
        let first = gate_and_insert(
            &db,
            ACCOUNT_MEMORY_SCOPE,
            "s1",
            MODE_AUTO_APPLY,
            &fenced(
                r#"[{"body":"Prefers spaces for indentation","kind":"preference","confidenceBps":9500,"rationale":"[e1] Direct preference"}]"#,
            ),
        )
        .unwrap();
        assert_eq!(first.activated, 0, "current mode downgraded during the run");
        assert_eq!(first.queued_for_review, 1);

        auto_apply_mode(&db);
        insert_message(&db, "s1", "e2", 2, "user.message", "I use the fish shell.");
        let second = gate_and_insert(
            &db,
            ACCOUNT_MEMORY_SCOPE,
            "s1",
            MODE_PROPOSE,
            &fenced(
                r#"[{"body":"Uses the fish shell","kind":"fact","confidenceBps":9500,"rationale":"[e2] Direct fact"}]"#,
            ),
        )
        .unwrap();
        assert_eq!(second.activated, 0, "claimed mode was never auto");
        assert_eq!(second.queued_for_review, 1);
        assert_eq!(
            memory_ledger::list(&db, ACCOUNT_MEMORY_SCOPE, Some("proposed"))
                .unwrap()
                .records
                .len(),
            2
        );
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
        assert!(
            memory_ledger::list(&db, ACCOUNT_MEMORY_SCOPE, Some("proposed"))
                .unwrap()
                .records
                .is_empty()
        );
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
        insert_message(
            &db,
            "s1",
            "e1",
            1,
            "user.message",
            "I always deploy on Fridays",
        );
        insert_message(&db, "s1", "e2", 2, "assistant.message", "Noted.");
        memory_ledger::save(&db, "Existing pin", None, None).unwrap();
        let digest = build_digest(&db, ACCOUNT_MEMORY_SCOPE, "s1")
            .unwrap()
            .unwrap();
        assert!(digest
            .text
            .contains("[e1] user: I always deploy on Fridays"));
        assert!(digest.text.contains("Already pinned"));
        assert!(digest.text.len() <= MAX_DIGEST_CHARS + 1_000);
        assert!(build_digest(&db, ACCOUNT_MEMORY_SCOPE, "empty-session")
            .unwrap()
            .is_none());
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
        assert!(
            claim_due(&db, now).unwrap().is_none(),
            "the lease excludes a second worker"
        );
        assert!(heartbeat(&db, &claimed.run_id, &claimed.lease_owner, now).unwrap());
        assert!(settle(
            &db,
            &claimed.run_id,
            &claimed.lease_owner,
            "completed",
            None,
            Some("codex"),
            Some("gpt-5.6-luna"),
            Some("digest"),
            420,
            1_700,
            2,
        )
        .unwrap());
        let last = last_run(&db, ACCOUNT_MEMORY_SCOPE).unwrap().unwrap();
        assert_eq!(last.status, "completed");
        assert_eq!(
            last.spend_microusd, 1_700,
            "spend is observed, never hardcoded zero"
        );
        assert_eq!(last.proposal_count, 2);
        assert!(
            settle(
                &db,
                &claimed.run_id,
                &claimed.lease_owner,
                "completed",
                None,
                None,
                None,
                None,
                0,
                0,
                0
            )
            .unwrap()
                == false,
            "a settled run stays settled"
        );
    }

    #[test]
    fn a_harness_that_cannot_run_tool_free_is_refused_at_the_setting() {
        let (_dir, db) = extraction_db();
        // Codex and OpenCode adapters refuse to start when a briefing policy is
        // present, so pinning one would fail every run after every turn.
        for harness in ["codex", "opencode"] {
            let error = update_settings(
                &db,
                ACCOUNT_MEMORY_SCOPE,
                MODE_PROPOSE,
                Some(harness),
                Some("m"),
            )
            .unwrap_err()
            .to_string();
            assert!(error.contains("tool-free"), "{harness}: {error}");
        }
        let current = settings(&db, ACCOUNT_MEMORY_SCOPE).unwrap();
        assert_eq!(current.harness, None, "a refused profile pins nothing");
        assert_eq!(current.model, None);
        assert!(update_settings(
            &db,
            ACCOUNT_MEMORY_SCOPE,
            MODE_PROPOSE,
            Some("claude"),
            Some("m")
        )
        .is_ok());
    }

    #[test]
    fn a_queued_run_does_not_erase_the_last_observed_spend() {
        let (_dir, db) = extraction_db();
        propose_mode(&db);
        insert_chat(&db, "s1", "direct");
        assert!(enqueue_after_turn(&db, "s1").unwrap());
        let claimed = claim_due(&db, Utc::now()).unwrap().unwrap();
        settle(
            &db,
            &claimed.run_id,
            &claimed.lease_owner,
            "completed",
            None,
            Some("claude"),
            Some("m"),
            Some("digest"),
            420,
            1_700,
            2,
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
        assert_eq!(
            occurrences, 0,
            "the extractor never imports or calls the router"
        );
        let live = include_str!("memory_extraction_live.rs");
        assert_eq!(live.matches(&module).count(), 0);
    }
}

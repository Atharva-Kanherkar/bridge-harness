//! The Work board: connected integration activity from the past 24 hours.
//!
//! [`board`] is the whole read path behind `work/get_work_board`, and it is
//! **store-only** by construction: it takes a `&Connection`, not a
//! `&Arc<BridgeCore>`, so it cannot reach an adapter map, a connector, or a
//! process spawner even by accident. The briefing runner reads integrations and
//! stores source activity dates separately from observation dates. Legacy local
//! fact projections remain available internally, but are never part of the board.
//!
//! The DTOs are `bridge_protocol::messages`' Work types used directly. A second
//! copy in this crate would only create something to drift.

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension};

use bridge_protocol::messages as wire;

use crate::{
    work_actions::{self, OpenTarget}, work_brief_store,
    work_connectors::ConnectorFamily, work_task_state::TaskState,
    BridgeError, WORKER_APPROVAL_TIMEOUT_SECONDS,
};

/// The defaults a board reports when Work has never been configured. Chosen to
/// match the epic's stated operational bounds: a 10-minute run deadline inside
/// a 15-minute lease, and a cooldown that cannot be shorter than the lease.
pub const DEFAULT_MAX_WALL_SECONDS: i64 = 600;
pub const DEFAULT_MAX_TURNS: i64 = 12;
pub const DEFAULT_MAX_TOOL_CALLS: i64 = 24;
pub const DEFAULT_COOLDOWN_MINUTES: i64 = 15;

/// The `work_fact_cache` kind holding base-branch divergence, keyed by
/// workspace id. Divergence needs a git subprocess to measure, which a
/// store-only read path cannot do, so it is observed elsewhere and read here.
pub const FACT_CACHE_BASE_DIVERGENCE: &str = "workspace_behind_base";

/// How long an observation counts as current. Past this the numbers still
/// describe something real, but they describe the past, and the board says so
/// rather than quietly presenting them as now.
pub const WORK_FACT_STALE_AFTER_SECONDS: i64 = 900;

/// Where Work's configuration lives in `configuration_entries`.
const SETTINGS_KIND: &str = "work";
const SETTINGS_ID: &str = "settings";

/// Work's configuration before anyone has configured it: facts only, no model,
/// no connector, no cadence.
pub fn default_settings() -> wire::WorkSettings {
    wire::WorkSettings {
        briefing: None,
        enabled_connector_instances: Vec::new(),
        refresh_on_focus: false,
        refresh_interval_minutes: None,
        cooldown_minutes: DEFAULT_COOLDOWN_MINUTES,
        limits: wire::WorkBriefLimits {
            max_wall_seconds: DEFAULT_MAX_WALL_SECONDS,
            max_turns: DEFAULT_MAX_TURNS,
            max_tool_calls: DEFAULT_MAX_TOOL_CALLS,
            max_output_tokens: None,
            cost_ceiling_microusd: None,
        },
    }
}

/// Read Work's settings. `Ok(None)` means never configured; `Err` means a row
/// exists that this contract cannot read, which the caller degrades rather than
/// failing the whole board over.
fn stored_settings(db: &Connection) -> Result<Option<wire::WorkSettings>, BridgeError> {
    // `optional()`, not `ok()`: an absent row means "never configured", while a
    // database error means the read failed and must not read as the same thing.
    let payload: Option<String> = db
        .query_row(
            "SELECT payload FROM configuration_entries WHERE kind=?1 AND id=?2",
            rusqlite::params![SETTINGS_KIND, SETTINGS_ID],
            |row| row.get(0),
        )
        .optional()?;
    let Some(payload) = payload else {
        return Ok(None);
    };
    serde_json::from_str(&payload)
        .map(Some)
        .map_err(|error| BridgeError::Invalid(format!("stored Work settings are invalid: {error}")))
}

/// The settings with their provenance: `configured: false` is a fresh install
/// reading defaults, `configured: true` with `briefing: None` is a user who
/// switched briefing off. The write path below is what keeps those two states
/// distinguishable.
pub fn read_settings(db: &Connection) -> Result<wire::WorkSettingsSnapshot, BridgeError> {
    Ok(match stored_settings(db)? {
        Some(settings) => wire::WorkSettingsSnapshot { configured: true, settings },
        None => wire::WorkSettingsSnapshot { configured: false, settings: default_settings() },
    })
}

/// The bounds the write path enforces. Constants rather than literals in the
/// checks, so the refusal messages and the rules cannot drift apart.
pub const MIN_REFRESH_INTERVAL_MINUTES: i64 = 15;
pub const MAX_REFRESH_INTERVAL_MINUTES: i64 = 1440;
pub const MAX_COOLDOWN_MINUTES: i64 = 1440;
pub const MIN_WALL_SECONDS: i64 = 60;
pub const MAX_WALL_SECONDS: i64 = 3600;
pub const MAX_TURNS_CEILING: i64 = 64;
pub const MAX_TOOL_CALLS_CEILING: i64 = 256;

/// Validate a settings payload. This is the authority — the frontend may
/// pre-empt an obvious mistake, but a payload that bypasses it is refused here
/// with the same rules.
pub fn validate_settings(settings: &wire::WorkSettings) -> Result<(), String> {
    validate_settings_keeping(settings, None)
}

/// Validate against what is already stored. `stored_briefing` is the profile
/// the database currently holds: submitting it back unchanged is not making a
/// choice, so it is exempt from the conformance gate. Without the exemption, a
/// harness that loses certification after being stored would freeze the whole
/// settings row — cadence, focus, even turning briefing off would be refused
/// over a profile the user is not changing. The run-time `resolve_briefing`
/// still refuses to *run* the uncertified profile, so nothing unsafe executes.
pub fn validate_settings_keeping(
    settings: &wire::WorkSettings,
    stored_briefing: Option<&wire::WorkBriefingProfile>,
) -> Result<(), String> {
    if !(0..=MAX_COOLDOWN_MINUTES).contains(&settings.cooldown_minutes) {
        return Err(format!(
            "cooldownMinutes must be between 0 and {MAX_COOLDOWN_MINUTES}"
        ));
    }
    if let Some(interval) = settings.refresh_interval_minutes {
        // The same floor the learning schedule enforces: a background model run
        // per minute is a subscription drain, not a cadence.
        if !(MIN_REFRESH_INTERVAL_MINUTES..=MAX_REFRESH_INTERVAL_MINUTES).contains(&interval) {
            return Err(format!(
                "refreshIntervalMinutes must be between {MIN_REFRESH_INTERVAL_MINUTES} and {MAX_REFRESH_INTERVAL_MINUTES}"
            ));
        }
    }
    let limits = &settings.limits;
    if !(MIN_WALL_SECONDS..=MAX_WALL_SECONDS).contains(&limits.max_wall_seconds) {
        return Err(format!(
            "maxWallSeconds must be between {MIN_WALL_SECONDS} and {MAX_WALL_SECONDS}"
        ));
    }
    if !(1..=MAX_TURNS_CEILING).contains(&limits.max_turns) {
        return Err(format!("maxTurns must be between 1 and {MAX_TURNS_CEILING}"));
    }
    if !(1..=MAX_TOOL_CALLS_CEILING).contains(&limits.max_tool_calls) {
        return Err(format!(
            "maxToolCalls must be between 1 and {MAX_TOOL_CALLS_CEILING}"
        ));
    }
    if limits.max_output_tokens.is_some_and(|value| value <= 0) {
        return Err("maxOutputTokens must be positive when set".into());
    }
    if limits.cost_ceiling_microusd.is_some_and(|value| value <= 0) {
        return Err("costCeilingMicrousd must be positive when set".into());
    }
    if settings
        .enabled_connector_instances
        .iter()
        .any(|instance| instance.trim().is_empty())
    {
        return Err("a connector instance id cannot be blank".into());
    }
    if let Some(briefing) = settings.briefing.as_ref() {
        if briefing.model.trim().is_empty() {
            return Err("the briefing model cannot be blank".into());
        }
        // The conformance gate is the authority on which harnesses may brief.
        // Refusing here keeps an unsupported harness out of storage entirely,
        // rather than storing it and skipping every run it would have caused.
        // The one exemption is the profile already stored, unchanged — see
        // `validate_settings_keeping`.
        if stored_briefing != Some(briefing) {
            crate::briefing_policy::adapter_may_brief(briefing.harness.as_str())
                .map_err(|unsupported| unsupported.reason())?;
        }
    }
    Ok(())
}

/// Persist Work's settings. The only production writer.
pub fn write_settings(
    db: &Connection,
    settings: &wire::WorkSettings,
) -> Result<wire::WorkSettingsSnapshot, BridgeError> {
    let stored = read_settings(db)?;
    let stored_briefing = stored.configured.then_some(stored.settings.briefing.as_ref()).flatten();
    validate_settings_keeping(settings, stored_briefing).map_err(BridgeError::Invalid)?;
    let payload = serde_json::to_string(settings)
        .map_err(|error| BridgeError::Invalid(format!("settings could not be serialised: {error}")))?;
    let now = Utc::now().to_rfc3339();
    db.execute(
        "INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at)
         VALUES(?1,?2,?3,?4,?4)
         ON CONFLICT(kind,id) DO UPDATE SET payload=excluded.payload,updated_at=excluded.updated_at",
        rusqlite::params![SETTINGS_KIND, SETTINGS_ID, payload, now],
    )?;
    Ok(wire::WorkSettingsSnapshot {
        configured: true,
        settings: settings.clone(),
    })
}

// ---------------------------------------------------------------------------
// Fact projection
// ---------------------------------------------------------------------------

/// How deep to walk a delegation chain looking for the session a human is
/// actually being asked something on. `MAX_DEPTH` for delegation is 3, so this
/// is generous; it exists to bound the recursion, not to express a policy.
const MAX_ANCESTOR_WALK: i64 = 16;

/// A projected fact plus the instant its timestamp parsed to. The instant is a
/// sort key only — the fact carries the text so a board never invents precision
/// a stored timestamp did not have.
struct Projected {
    fact: wire::WorkFact,
    at: DateTime<Utc>,
}

fn instant(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|parsed| parsed.with_timezone(&Utc))
}

fn projected(fact: wire::WorkFact) -> Projected {
    // An unparseable timestamp sorts last within its severity rather than
    // first: it is unknown, and unknown is not "oldest".
    let at = instant(&fact.actionable_at).unwrap_or(DateTime::<Utc>::MAX_UTC);
    Projected { fact, at }
}

/// Sort into the board's documented order and drop repeats.
///
/// Severity, then oldest actionable first, then key. The key tie-break is what
/// makes the order total: two facts can never compare equal, so the board does
/// not depend on the order SQLite happened to return rows in.
fn finalize(mut projections: Vec<Projected>) -> Vec<wire::WorkFact> {
    projections.sort_by(|left, right| {
        left.fact
            .severity
            .cmp(&right.fact.severity)
            .then(left.at.cmp(&right.at))
            .then(left.fact.dedupe_key.cmp(&right.fact.dedupe_key))
    });
    let mut seen = std::collections::HashSet::new();
    projections
        .into_iter()
        .filter(|projection| seen.insert(projection.fact.dedupe_key.clone()))
        .map(|projection| projection.fact)
        .collect()
}

/// Failed completion checks on attempts nobody has resolved.
///
/// An attempt that is verified, waived, or superseded is settled; its old failed
/// check rows are history, not work. A required check blocks — the completion
/// gate will not pass without it — while an optional one only warrants a look.
fn failed_completion_checks(db: &Connection) -> Result<Vec<Projected>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT c.attempt_id,c.check_id,c.required,c.detail,
                COALESCE(c.completed_at,c.started_at,a.started_at),a.session_id
           FROM eval_check_runs c
           JOIN eval_attempts a ON a.id=c.attempt_id
          WHERE c.status='failed'
            AND a.status NOT IN ('verified','waived','superseded')",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, bool>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
        ))
    })?;
    let mut facts = Vec::new();
    for row in rows {
        let (attempt_id, check_id, required, detail, at, session_id) = row?;
        facts.push(projected(wire::WorkFact {
            kind: wire::WorkFactKind::FailedCompletionCheck,
            dedupe_key: format!("completion-check:{attempt_id}:{check_id}"),
            severity: if required {
                wire::WorkFactSeverity::Blocking
            } else {
                wire::WorkFactSeverity::Attention
            },
            title: format!("{check_id} failed"),
            detail,
            target: wire::WorkFactTarget::CompletionAttempt {
                session_id: session_id.clone(),
                attempt_id: attempt_id.clone(),
            },
            actionable_at: at.clone(),
            observed_at: at,
            // The row *is* the observation; there is nothing cached about it.
            freshness: wire::WorkFactFreshness::Live,
            action: wire::WorkFactAction::ReviewCompletionCheck {
                session_id,
                attempt_id,
                check_id,
            },
        }));
    }
    Ok(facts)
}

/// Queued workers parked because someone upstream is waiting on a human.
///
/// The action points at the session the approval is actually on, found by
/// walking `parent_session_id` up to the nearest `waiting` ancestor. Pointing it
/// at the queue row would be useless: a queue row is not something a person
/// answers.
fn blocked_queue_items(db: &Connection) -> Result<Vec<Projected>, BridgeError> {
    let mut chain = db.prepare(
        "WITH RECURSIVE chain(queue_id,session_id,depth) AS (
             SELECT id,parent_session_id,0 FROM worker_queue WHERE queue_status='blocked_on_human'
             UNION ALL
             SELECT c.queue_id,s.parent_session_id,c.depth+1
               FROM chain c JOIN sessions s ON s.id=c.session_id
              WHERE s.parent_session_id IS NOT NULL AND c.depth < ?1
         )
         SELECT c.queue_id,c.session_id
           FROM chain c JOIN sessions s ON s.id=c.session_id
          WHERE s.status='waiting'
          ORDER BY c.queue_id,c.depth",
    )?;
    let mut waiting: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let rows = chain.query_map([MAX_ANCESTOR_WALK], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (queue_id, session_id) = row?;
        // Ordered by depth, so the first row for a queue item is the nearest.
        waiting.entry(queue_id).or_insert(session_id);
    }

    let mut statement = db.prepare(
        "SELECT q.id,q.workspace_id,q.parent_session_id,COALESCE(q.blocked_at,q.updated_at),
                COALESCE(s.label,q.parent_session_id)
           FROM worker_queue q
           JOIN sessions s ON s.id=q.parent_session_id
          WHERE q.queue_status='blocked_on_human'",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;
    let mut facts = Vec::new();
    for row in rows {
        let (queue_id, workspace_id, parent_session_id, at, label) = row?;
        // No `waiting` ancestor recorded means the queue and the session tree
        // disagree. Send the human to the parent rather than nowhere.
        let session_id = waiting
            .get(&queue_id)
            .cloned()
            .unwrap_or_else(|| parent_session_id.clone());
        facts.push(projected(wire::WorkFact {
            kind: wire::WorkFactKind::BlockedWorkerQueueItem,
            dedupe_key: format!("worker-queue:{queue_id}"),
            // Queued work is stopped, and only a person restarts it.
            severity: wire::WorkFactSeverity::Blocking,
            title: format!("A queued worker for {label} is waiting on you"),
            detail: Some(
                "the delegation cannot start until the approval above it is answered".to_owned(),
            ),
            target: wire::WorkFactTarget::WorkerQueueItem {
                queue_id,
                workspace_id,
            },
            actionable_at: at.clone(),
            observed_at: at,
            freshness: wire::WorkFactFreshness::Live,
            action: wire::WorkFactAction::AnswerApproval {
                session_id,
                approval_sequence: None,
            },
        }));
    }
    Ok(facts)
}

/// Approval requests nobody has answered.
///
/// Two resolution shapes reach the store: adapter approvals carry
/// `requestEventId` (nested under the event envelope's `data`, or top-level),
/// and policy approvals carry `approvalId`. An entry is unresolved when neither
/// match exists.
///
/// Provider approvals require a live session. Prompt proposals change future
/// role defaults and remain reviewable after the originating adapter stops.
fn actionable_approvals(db: &Connection, now: DateTime<Utc>) -> Result<Vec<Projected>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT e.session_id,e.sequence,e.created_at,
                COALESCE(json_extract(e.payload,'$.title'),
                         json_extract(e.payload,'$.data.title'),
                         'Approval required')
           FROM session_entries e
           JOIN sessions s ON s.id=e.session_id
          WHERE e.kind='approval.requested'
            AND (s.status NOT IN ('stopped','failed','completed','cancelled')
                 OR json_extract(e.payload,'$.approvalType')='prompt_mutation')
            AND NOT EXISTS (
                SELECT 1 FROM session_entries r
                 WHERE r.session_id=e.session_id AND r.kind='approval.resolved'
                   AND (COALESCE(json_extract(r.payload,'$.data.requestEventId'),
                                 json_extract(r.payload,'$.requestEventId')) = e.sequence
                     OR (COALESCE(json_extract(e.payload,'$.approvalId'),
                                  json_extract(e.payload,'$.data.approvalId')) IS NOT NULL
                         AND COALESCE(json_extract(r.payload,'$.approvalId'),
                                      json_extract(r.payload,'$.data.approvalId'))
                           = COALESCE(json_extract(e.payload,'$.approvalId'),
                                      json_extract(e.payload,'$.data.approvalId'))))
            )",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    let mut facts = Vec::new();
    for row in rows {
        let (session_id, sequence, at, title) = row?;
        // Past the approval deadline the thing it was holding has already been
        // given up on, so it is not merely waiting — it is overdue.
        let expired = instant(&at).is_some_and(|requested| {
            now.signed_duration_since(requested).num_seconds() >= WORKER_APPROVAL_TIMEOUT_SECONDS
        });
        facts.push(projected(wire::WorkFact {
            kind: wire::WorkFactKind::ActionableApproval,
            dedupe_key: format!("approval:{session_id}:{sequence}"),
            severity: if expired {
                wire::WorkFactSeverity::Blocking
            } else {
                wire::WorkFactSeverity::Attention
            },
            title,
            detail: expired.then(|| {
                format!(
                    "unanswered for more than {} minutes",
                    WORKER_APPROVAL_TIMEOUT_SECONDS / 60
                )
            }),
            target: wire::WorkFactTarget::Session {
                session_id: session_id.clone(),
            },
            actionable_at: at.clone(),
            observed_at: at,
            freshness: wire::WorkFactFreshness::Live,
            action: wire::WorkFactAction::AnswerApproval {
                session_id,
                approval_sequence: Some(sequence),
            },
        }));
    }
    Ok(facts)
}

/// Workspaces that have drifted behind their base branch, read from
/// `work_fact_cache`.
///
/// This never calls git. A row's `observed_at` is the whole truth about how
/// current it is: inside the staleness window it is `live`, outside it is
/// `stale`, and a failed observation is `unknown`. A workspace with **no** row
/// projects nothing at all — Bridge has not looked, and rendering "up to date"
/// for something never measured is the failure this cache exists to prevent.
fn workspaces_behind_base(db: &Connection, now: DateTime<Utc>) -> Result<Vec<Projected>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT c.cache_key,c.status,c.payload,c.detail,c.observed_at,w.title,
                (SELECT s.id FROM sessions s WHERE s.workspace_id=c.cache_key
                  ORDER BY CASE WHEN s.status IN ('stopped','failed','completed','cancelled')
                                THEN 1 ELSE 0 END,
                           s.started_at DESC,s.id
                  LIMIT 1)
           FROM work_fact_cache c
           JOIN workspaces w ON w.id=c.cache_key
          WHERE c.kind=?1",
    )?;
    let rows = statement.query_map([FACT_CACHE_BASE_DIVERGENCE], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, Option<String>>(6)?,
        ))
    })?;
    let mut facts = Vec::new();
    for row in rows {
        let (workspace_id, status, payload, detail, observed_at, title, session_id) = row?;
        // The only local action on divergence resolves a session's repository, so
        // a workspace with no session has nothing to offer and is not surfaced.
        let Some(session_id) = session_id else { continue };

        let measured = (status == "ok")
            .then_some(payload.as_deref())
            .flatten()
            .and_then(|payload| {
                serde_json::from_str::<crate::git::BaseBranchDivergence>(payload).ok()
            });
        let (freshness, summary) = match measured {
            // Written as `ok` by a binary from before an unavailable comparison
            // was recorded as a failure. It means the same thing: no comparison
            // was obtained, so nothing is claimed.
            Some(divergence) if divergence.unavailable_reason.is_some() => {
                (wire::WorkFactFreshness::Unknown, divergence.summary())
            }
            Some(divergence) => {
                if !divergence.should_warn() {
                    // Close enough to its base to be nobody's problem.
                    continue;
                }
                let stale = instant(&observed_at).is_none_or(|seen| {
                    now.signed_duration_since(seen).num_seconds() >= WORK_FACT_STALE_AFTER_SECONDS
                });
                (
                    if stale {
                        wire::WorkFactFreshness::Stale
                    } else {
                        wire::WorkFactFreshness::Live
                    },
                    divergence.summary(),
                )
            }
            // Either the observation failed, or it succeeded and stored something
            // this binary cannot read. Both mean the same thing to a reader:
            // nothing is known, so nothing is claimed.
            None => (
                wire::WorkFactFreshness::Unknown,
                detail.unwrap_or_else(|| "the last measurement did not complete".to_owned()),
            ),
        };

        facts.push(projected(wire::WorkFact {
            kind: wire::WorkFactKind::WorkspaceBehindBase,
            dedupe_key: format!("workspace-base:{workspace_id}"),
            severity: wire::WorkFactSeverity::Attention,
            title: match freshness {
                wire::WorkFactFreshness::Unknown => {
                    format!("{title} could not be measured against its base branch")
                }
                _ => format!("{title} has drifted behind its base branch"),
            },
            detail: Some(summary),
            target: wire::WorkFactTarget::Workspace {
                workspace_id: workspace_id.clone(),
                session_id: Some(session_id.clone()),
            },
            actionable_at: observed_at.clone(),
            observed_at,
            freshness,
            // A stale or unreadable observation is not something to act on. Offer
            // to measure again; do not offer to fast-forward onto a number
            // nobody has checked recently.
            action: match freshness {
                wire::WorkFactFreshness::Live => wire::WorkFactAction::RefreshWorkspaceBase {
                    session_id,
                    workspace_id,
                },
                _ => wire::WorkFactAction::RefreshBaseObservation {
                    session_id,
                    workspace_id,
                },
            },
        }));
    }
    Ok(facts)
}

/// Every fact, ordered and deduplicated. Store-only.
pub fn facts(db: &Connection, now: DateTime<Utc>) -> Result<Vec<wire::WorkFact>, BridgeError> {
    let mut projections = failed_completion_checks(db)?;
    projections.extend(blocked_queue_items(db)?);
    projections.extend(actionable_approvals(db, now)?);
    projections.extend(workspaces_behind_base(db, now)?);
    Ok(finalize(projections))
}

/// The board. Deterministic, offline, and useful with no model configured.
fn wire_task_state(state: TaskState) -> wire::WorkTaskState {
    match state {
        TaskState::Active => wire::WorkTaskState::Active,
        TaskState::Snoozed => wire::WorkTaskState::Snoozed,
        TaskState::Done => wire::WorkTaskState::Done,
        TaskState::Dismissed => wire::WorkTaskState::Dismissed,
        TaskState::Stale => wire::WorkTaskState::Stale,
    }
}

fn checked_evidence_target(stored: Option<&str>, source_kind: &str) -> Option<wire::WorkEvidenceTarget> {
    let family = ConnectorFamily::parse(source_kind.split('.').next()?)?;
    match work_actions::open_target(stored, family).ok()? {
        OpenTarget::External { url, host } => Some(wire::WorkEvidenceTarget::ExternalLink { url, host }),
        OpenTarget::Session { session_id } => Some(wire::WorkEvidenceTarget::Session { session_id }),
    }
}

/// Project durable tasks without touching connectors or providers. Expired snoozes are
/// reactivated before visibility is decided, so a read reveals them immediately.
fn suggested_tasks(db: &Connection, now: DateTime<Utc>) -> Result<Vec<wire::WorkTask>, BridgeError> {
    let now_text = now.to_rfc3339();
    db.execute(
        "UPDATE work_tasks
            SET state=CASE WHEN miss_count >= 2 THEN 'stale' ELSE 'active' END,
                snoozed_until=NULL,resolved_at=NULL,updated_at=?1
          WHERE state='snoozed' AND snoozed_until IS NOT NULL
            AND julianday(snoozed_until) <= julianday(?1)",
        rusqlite::params![now_text],
    )?;
    let mut statement = db.prepare(
        "SELECT id,fingerprint,connector_instance_id,canonical_resource_id,source_kind,
                title,why,rank,confidence_bps,state,pinned,snoozed_until,evidence_digest,
                evidence_target,evidence_observed_at,miss_count,workspace_id,created_at,updated_at,source_activity_at
           FROM work_tasks
          WHERE state='active'
            AND julianday(source_activity_at) >= julianday(?1) - 1
            AND julianday(source_activity_at) <= julianday(?1)
            AND source_kind IN ('slack.message','github.item','gmail.thread','linear.issue','notion.page')
            AND connector_instance_id != ''
          ORDER BY julianday(source_activity_at) DESC,id
          LIMIT 100",
    )?;
    let rows = statement.query_map([&now_text], |row| {
        Ok((
            row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?,
            row.get::<_, String>(6)?, row.get::<_, i64>(7)?, row.get::<_, i64>(8)?,
            row.get::<_, String>(9)?, row.get::<_, i64>(10)? != 0, row.get::<_, Option<String>>(11)?,
            row.get::<_, Option<String>>(12)?, row.get::<_, Option<String>>(13)?,
            row.get::<_, Option<String>>(14)?, row.get::<_, i64>(15)?,
            row.get::<_, Option<String>>(16)?, row.get::<_, String>(17)?, row.get::<_, String>(18)?, row.get::<_, Option<String>>(19)?,
        ))
    })?;
    let mut tasks = Vec::new();
    for row in rows {
        let (id, fingerprint, connector_instance_id, canonical_resource_id, source_kind, title, why,
            rank, confidence_bps, stored_state, pinned, snoozed_until, evidence_digest,
            evidence_target, evidence_observed_at, miss_count, workspace_id, created_at,
            updated_at, source_activity_at) = row?;
        let state = TaskState::parse(&stored_state);
        if !state.visible(pinned) && !matches!(state, TaskState::Snoozed | TaskState::Dismissed) {
            continue;
        }
        tasks.push(wire::WorkTask {
            id, fingerprint, connector_instance_id, canonical_resource_id,
            source_kind: source_kind.clone(), title, why, rank, confidence_bps,
            state: wire_task_state(state), pinned, snoozed_until, evidence_digest,
            evidence_target: checked_evidence_target(evidence_target.as_deref(), &source_kind),
            evidence_observed_at, source_activity_at, miss_count, workspace_id, created_at, updated_at,
        });
    }
    Ok(tasks)
}

pub fn board(db: &Connection) -> Result<wire::WorkBoard, BridgeError> {
    let (settings, settings_error) = match stored_settings(db) {
        Ok(Some(settings)) => (settings, None),
        Ok(None) => (default_settings(), None),
        // A configuration row we cannot read is worth saying out loud, but it
        // is not worth withholding every fact over: the facts do not depend on
        // it. Fall back to defaults and let `suggestions` carry the reason.
        Err(_) => (
            default_settings(),
            Some("stored Work settings could not be read; using defaults".to_owned()),
        ),
    };

    let latest_run = work_brief_store::latest_run(db)?;
    let sources = match latest_run.as_ref() {
        Some(run) => work_brief_store::read_coverage(db, &run.id)?,
        None => Vec::new(),
    };
    let usage = latest_run.as_ref().and_then(|run| run.usage.clone());
    let suggestions = match settings_error {
        Some(detail) => wire::WorkSuggestions {
            state: wire::WorkSuggestionsState::Degraded,
            detail: Some(detail),
        },
        None if settings.briefing.is_none() => wire::WorkSuggestions {
            state: wire::WorkSuggestionsState::NotConfigured,
            detail: Some(
                "Set up a briefing model in Settings → Work to read your connected integrations.".to_owned(),
            ),
        },
        None => match latest_run.as_ref().map(|run| run.status) {
            Some(wire::WorkBriefRunStatus::Running) => wire::WorkSuggestions {
                state: wire::WorkSuggestionsState::Running, detail: None,
            },
            Some(wire::WorkBriefRunStatus::Failed | wire::WorkBriefRunStatus::Cancelled) => wire::WorkSuggestions {
                state: wire::WorkSuggestionsState::Degraded,
                detail: latest_run.as_ref().and_then(|run| run.failure_detail.clone()),
            },
            _ => wire::WorkSuggestions { state: wire::WorkSuggestionsState::Ready, detail: None },
        },
    };

    let now = Utc::now();
    Ok(wire::WorkBoard {
        generated_at: now.to_rfc3339(),
        facts: Vec::new(),
        tasks: suggested_tasks(db, now)?,
        latest_run,
        sources,
        usage,
        settings,
        suggestions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;

    fn memory_db() -> Connection {
        store::open(std::path::Path::new(":memory:")).unwrap()
    }

    fn store_settings(db: &Connection, payload: &str) {
        db.execute(
            "INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at)
             VALUES(?1,?2,?3,'now','now')",
            rusqlite::params![SETTINGS_KIND, SETTINGS_ID, payload],
        )
        .unwrap();
    }

    fn claude_briefing() -> wire::WorkBriefingProfile {
        wire::WorkBriefingProfile {
            harness: wire::HarnessId::parse("claude").unwrap(),
            model: "haiku".into(),
            effort: None,
        }
    }

    #[test]
    fn a_fresh_install_reads_defaults_and_is_not_configured() {
        let db = memory_db();
        let snapshot = read_settings(&db).unwrap();
        assert!(!snapshot.configured);
        assert_eq!(snapshot.settings, default_settings());
    }

    #[test]
    fn written_settings_round_trip_and_a_rewrite_is_an_upsert() {
        let db = memory_db();
        let mut settings = default_settings();
        settings.briefing = Some(claude_briefing());
        settings.refresh_on_focus = true;
        settings.refresh_interval_minutes = Some(60);
        let written = write_settings(&db, &settings).unwrap();
        assert!(written.configured);

        let read = read_settings(&db).unwrap();
        assert!(read.configured);
        assert_eq!(read.settings, settings);

        // A second write updates the same row rather than erroring on the key.
        settings.refresh_interval_minutes = Some(30);
        write_settings(&db, &settings).unwrap();
        assert_eq!(read_settings(&db).unwrap().settings.refresh_interval_minutes, Some(30));
    }

    #[test]
    fn briefing_switched_off_is_not_the_same_stored_state_as_never_configured() {
        let db = memory_db();
        let off = default_settings();
        assert!(off.briefing.is_none(), "off is expressed as a stored row with no profile");
        write_settings(&db, &off).unwrap();
        let stored = read_settings(&db).unwrap();
        assert!(stored.configured, "the row exists: the user chose this");
        assert!(stored.settings.briefing.is_none());

        let fresh = read_settings(&memory_db()).unwrap();
        assert!(!fresh.configured, "no row: nobody chose anything");
        assert_ne!(stored, fresh);
    }

    #[test]
    fn validation_refuses_each_out_of_bounds_field_with_a_reason() {
        let base = default_settings();

        let mut cooldown = base.clone();
        cooldown.cooldown_minutes = MAX_COOLDOWN_MINUTES + 1;
        assert!(validate_settings(&cooldown).unwrap_err().contains("cooldownMinutes"));

        let mut interval = base.clone();
        interval.refresh_interval_minutes = Some(MIN_REFRESH_INTERVAL_MINUTES - 1);
        assert!(validate_settings(&interval).unwrap_err().contains("refreshIntervalMinutes"));

        let mut wall = base.clone();
        wall.limits.max_wall_seconds = MAX_WALL_SECONDS + 1;
        assert!(validate_settings(&wall).unwrap_err().contains("maxWallSeconds"));

        let mut turns = base.clone();
        turns.limits.max_turns = 0;
        assert!(validate_settings(&turns).unwrap_err().contains("maxTurns"));

        let mut calls = base.clone();
        calls.limits.max_tool_calls = MAX_TOOL_CALLS_CEILING + 1;
        assert!(validate_settings(&calls).unwrap_err().contains("maxToolCalls"));

        let mut tokens = base.clone();
        tokens.limits.max_output_tokens = Some(0);
        assert!(validate_settings(&tokens).unwrap_err().contains("maxOutputTokens"));

        let mut cost = base.clone();
        cost.limits.cost_ceiling_microusd = Some(-1);
        assert!(validate_settings(&cost).unwrap_err().contains("costCeilingMicrousd"));

        let mut blank_instance = base.clone();
        blank_instance.enabled_connector_instances = vec!["  ".into()];
        assert!(validate_settings(&blank_instance).unwrap_err().contains("connector instance"));

        let mut blank_model = base.clone();
        blank_model.briefing = Some(wire::WorkBriefingProfile {
            harness: wire::HarnessId::parse("claude").unwrap(),
            model: "   ".into(),
            effort: None,
        });
        assert!(validate_settings(&blank_model).unwrap_err().contains("model"));
    }

    #[test]
    fn an_uncertified_harness_is_refused_at_write_time_with_the_gates_reason() {
        // The frontend offers only certified harnesses, but the frontend is not
        // the authority: a payload naming codex directly is refused here.
        let db = memory_db();
        let mut settings = default_settings();
        settings.briefing = Some(wire::WorkBriefingProfile {
            harness: wire::HarnessId::parse("codex").unwrap(),
            model: "gpt-5.6-luna".into(),
            effort: None,
        });
        let refused = write_settings(&db, &settings).unwrap_err();
        let reason = refused.to_string();
        assert!(reason.contains("codex"), "{reason}");
        assert!(
            !read_settings(&db).unwrap().configured,
            "a refused write stores nothing"
        );
    }

    #[test]
    fn a_stored_profile_that_lost_certification_does_not_freeze_the_settings_row() {
        // A profile can be certified when stored and uncertified later. Keeping
        // it unchanged is not making a choice, so cadence edits and turning the
        // briefing off must still write — only naming it anew is gated.
        let db = memory_db();
        let uncertified = wire::WorkBriefingProfile {
            harness: wire::HarnessId::parse("codex").unwrap(),
            model: "gpt-5.6-luna".into(),
            effort: None,
        };
        let mut stored = default_settings();
        stored.briefing = Some(uncertified.clone());
        // Planted directly, simulating certification lost after storage: the
        // production writer would have accepted this while the gate passed.
        db.execute(
            "INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at)
             VALUES(?1,?2,?3,'2026-08-19T00:00:00+00:00','2026-08-19T00:00:00+00:00')",
            rusqlite::params![
                SETTINGS_KIND,
                SETTINGS_ID,
                serde_json::to_string(&stored).unwrap()
            ],
        )
        .unwrap();

        // Changing the cadence while keeping the stored profile writes.
        let mut cadence_edit = stored.clone();
        cadence_edit.refresh_interval_minutes = Some(60);
        write_settings(&db, &cadence_edit).unwrap();
        assert_eq!(
            read_settings(&db).unwrap().settings.refresh_interval_minutes,
            Some(60)
        );

        // Turning the briefing off writes: null names no harness at all.
        let mut off = cadence_edit.clone();
        off.briefing = None;
        write_settings(&db, &off).unwrap();
        assert_eq!(read_settings(&db).unwrap().settings.briefing, None);

        // Naming the uncertified harness *again* is a new choice, and refused.
        let mut renamed = off;
        renamed.briefing = Some(uncertified);
        let refused = write_settings(&db, &renamed).unwrap_err();
        assert!(refused.to_string().contains("codex"), "{refused}");
    }

    #[test]
    fn an_invalid_write_never_reaches_storage() {
        let db = memory_db();
        let mut settings = default_settings();
        settings.briefing = Some(claude_briefing());
        write_settings(&db, &settings).unwrap();

        let mut broken = settings.clone();
        broken.limits.max_turns = 0;
        assert!(write_settings(&db, &broken).is_err());
        assert_eq!(
            read_settings(&db).unwrap().settings,
            settings,
            "the stored settings are the last valid write"
        );
    }

    /// A project, workspace, and session, so the foreign keys the projections
    /// join through are satisfiable.
    fn seed(db: &Connection) {
        db.execute_batch(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Bridge','/tmp/p','now');
             INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at)
                 VALUES('w','p','Kyoto','Task','bridge/task','/tmp/w','idle','now');
             INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source)
                 VALUES('parent','w','codex','Orchestrator','working','reported');",
        )
        .unwrap();
    }

    fn add_session(db: &Connection, id: &str, parent: Option<&str>, status: &str) {
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id)
             VALUES(?1,'w','codex',?1,?2,'reported',?3)",
            rusqlite::params![id, status, parent],
        )
        .unwrap();
    }

    /// One completion attempt with one check in the given state.
    fn add_check(
        db: &Connection,
        attempt: &str,
        check: &str,
        required: bool,
        status: &str,
        attempt_status: &str,
        completed_at: &str,
    ) {
        db.execute(
            "INSERT OR IGNORE INTO completion_contracts(id,workspace_id,session_id,schema_version,
                 acceptance_criteria,markdown_committed,status,created_at,updated_at)
             VALUES('c','w','parent',1,'[]',0,'open','now','now')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT OR IGNORE INTO eval_plans(id,contract_id,schema_version,risk,plan,created_at)
             VALUES('plan','c',1,'low','{}','now')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT OR IGNORE INTO eval_attempts(id,plan_id,session_id,repository_head,dirty_digest,
                 repository_path,status,started_at)
             VALUES(?1,'plan','parent','head','digest','/tmp/w',?2,'2026-08-19T08:00:00+00:00')",
            rusqlite::params![attempt, attempt_status],
        )
        .unwrap();
        db.execute(
            "INSERT INTO eval_check_runs(id,attempt_id,check_id,kind,required,status,executor,
                 detail,artifact_refs,completed_at)
             VALUES(?1,?2,?3,'deterministic',?4,?5,'shell','2 failing tests','[]',?6)",
            rusqlite::params![
                format!("{attempt}:{check}"),
                attempt,
                check,
                required,
                status,
                completed_at
            ],
        )
        .unwrap();
    }

    fn add_blocked_queue_item(db: &Connection, id: &str, parent: &str, blocked_at: &str) {
        db.execute(
            "INSERT INTO worker_queue(id,parent_session_id,workspace_id,turn_id,request,actual_model,
                 queue_status,blocked_at,created_at,updated_at)
             VALUES(?1,?2,'w','turn','{}','model','blocked_on_human',?3,'now','now')",
            rusqlite::params![id, parent, blocked_at],
        )
        .unwrap();
    }

    fn add_entry(db: &Connection, session: &str, sequence: i64, kind: &str, payload: &str) {
        db.execute(
            "INSERT INTO session_entries(id,session_id,sequence,kind,payload,created_at)
             VALUES(?1,?2,?3,?4,?5,?6)",
            rusqlite::params![
                format!("{session}-{sequence}"),
                session,
                sequence,
                kind,
                payload,
                "2026-08-19T09:00:00+00:00"
            ],
        )
        .unwrap();
    }

    fn now() -> DateTime<Utc> {
        instant("2026-08-19T09:10:00+00:00").unwrap()
    }

    fn keys(facts: &[wire::WorkFact]) -> Vec<&str> {
        facts.iter().map(|fact| fact.dedupe_key.as_str()).collect()
    }

    #[test]
    fn an_empty_database_projects_no_facts() {
        let db = memory_db();
        assert!(facts(&db, now()).unwrap().is_empty());
        assert!(board(&db).unwrap().facts.is_empty());
    }

    #[test]
    fn a_required_failed_check_blocks_and_an_optional_one_asks_for_attention() {
        let db = memory_db();
        seed(&db);
        add_check(&db, "a-1", "cargo-test", true, "failed", "running", "2026-08-19T08:30:00+00:00");
        add_check(&db, "a-1", "clippy", false, "failed", "running", "2026-08-19T08:31:00+00:00");
        let facts = facts(&db, now()).unwrap();
        assert_eq!(
            keys(&facts),
            vec!["completion-check:a-1:cargo-test", "completion-check:a-1:clippy"]
        );
        assert_eq!(facts[0].severity, wire::WorkFactSeverity::Blocking);
        assert_eq!(facts[1].severity, wire::WorkFactSeverity::Attention);
        assert_eq!(facts[0].kind, wire::WorkFactKind::FailedCompletionCheck);
        assert_eq!(facts[0].actionable_at, "2026-08-19T08:30:00+00:00");
        assert_eq!(facts[0].freshness, wire::WorkFactFreshness::Live);
        assert_eq!(
            facts[0].target,
            wire::WorkFactTarget::CompletionAttempt {
                session_id: "parent".into(),
                attempt_id: "a-1".into()
            }
        );
        assert_eq!(
            facts[0].action,
            wire::WorkFactAction::ReviewCompletionCheck {
                session_id: "parent".into(),
                attempt_id: "a-1".into(),
                check_id: "cargo-test".into(),
            },
            "a failed check is reviewed, never cleared"
        );
    }

    #[test]
    fn a_passing_check_and_a_settled_attempt_project_nothing() {
        let db = memory_db();
        seed(&db);
        add_check(&db, "a-1", "cargo-test", true, "passed", "verified", "2026-08-19T08:30:00+00:00");
        assert!(facts(&db, now()).unwrap().is_empty());

        for settled in ["verified", "waived", "superseded"] {
            let db = memory_db();
            seed(&db);
            add_check(
                &db,
                "a-1",
                "cargo-test",
                true,
                "failed",
                settled,
                "2026-08-19T08:30:00+00:00",
            );
            assert!(
                facts(&db, now()).unwrap().is_empty(),
                "a {settled} attempt's failed checks are history, not work"
            );
        }
    }

    #[test]
    fn a_blocked_queue_item_points_at_the_nearest_waiting_ancestor() {
        let db = memory_db();
        seed(&db);
        // parent (working) -> middle (waiting) -> leaf (working); the queue item
        // hangs off the leaf, so the human is being asked on `middle`.
        add_session(&db, "middle", Some("parent"), "waiting");
        add_session(&db, "leaf", Some("middle"), "working");
        add_blocked_queue_item(&db, "q-1", "leaf", "2026-08-19T08:00:00+00:00");
        let facts = facts(&db, now()).unwrap();
        assert_eq!(keys(&facts), vec!["worker-queue:q-1"]);
        assert_eq!(facts[0].severity, wire::WorkFactSeverity::Blocking);
        assert_eq!(
            facts[0].target,
            wire::WorkFactTarget::WorkerQueueItem {
                queue_id: "q-1".into(),
                workspace_id: "w".into()
            }
        );
        assert_eq!(
            facts[0].action,
            wire::WorkFactAction::AnswerApproval {
                session_id: "middle".into(),
                approval_sequence: None,
            },
            "the action must land on the session the approval is on, not the queue row"
        );
    }

    #[test]
    fn a_blocked_queue_item_with_no_waiting_ancestor_falls_back_to_its_parent() {
        let db = memory_db();
        seed(&db);
        add_blocked_queue_item(&db, "q-1", "parent", "2026-08-19T08:00:00+00:00");
        let facts = facts(&db, now()).unwrap();
        assert_eq!(
            facts[0].action,
            wire::WorkFactAction::AnswerApproval {
                session_id: "parent".into(),
                approval_sequence: None,
            },
            "the queue and the session tree disagreeing must not produce an action to nowhere"
        );
    }

    #[test]
    fn a_queue_item_that_is_merely_queued_is_not_a_fact() {
        let db = memory_db();
        seed(&db);
        db.execute(
            "INSERT INTO worker_queue(id,parent_session_id,workspace_id,turn_id,request,actual_model,
                 queue_status,created_at,updated_at)
             VALUES('q-1','parent','w','turn','{}','model','queued','now','now')",
            [],
        )
        .unwrap();
        assert!(facts(&db, now()).unwrap().is_empty());
    }

    #[test]
    fn a_young_approval_asks_for_attention_and_an_overdue_one_blocks() {
        let db = memory_db();
        seed(&db);
        add_entry(
            &db,
            "parent",
            4,
            "approval.requested",
            r#"{"title":"Approve command","status":"pending"}"#,
        );
        let young = facts(&db, now()).unwrap();
        assert_eq!(keys(&young), vec!["approval:parent:4"]);
        assert_eq!(young[0].severity, wire::WorkFactSeverity::Attention);
        assert_eq!(young[0].title, "Approve command");
        assert!(young[0].detail.is_none());
        assert_eq!(
            young[0].action,
            wire::WorkFactAction::AnswerApproval {
                session_id: "parent".into(),
                approval_sequence: Some(4),
            }
        );

        // The same request, read an hour after it arrived — past the 30-minute
        // approval deadline.
        let overdue = facts(&db, instant("2026-08-19T10:00:00+00:00").unwrap()).unwrap();
        assert_eq!(overdue[0].severity, wire::WorkFactSeverity::Blocking);
        assert!(overdue[0].detail.as_deref().is_some_and(|d| d.contains("30 minutes")));
    }

    #[test]
    fn a_resolved_approval_projects_nothing_in_either_resolution_shape() {
        // Adapter shape: the resolution names the request's sequence, nested in
        // the normalized event envelope.
        let db = memory_db();
        seed(&db);
        add_entry(&db, "parent", 4, "approval.requested", r#"{"title":"Approve command"}"#);
        add_entry(
            &db,
            "parent",
            5,
            "approval.resolved",
            r#"{"data":{"requestEventId":4,"decision":"accept"}}"#,
        );
        assert!(facts(&db, now()).unwrap().is_empty());

        // Policy shape: the resolution names the approval id instead.
        let db = memory_db();
        seed(&db);
        add_entry(
            &db,
            "parent",
            6,
            "approval.requested",
            r#"{"title":"Approve delegation write scope","approvalId":"scope-1"}"#,
        );
        add_entry(&db, "parent", 7, "approval.resolved", r#"{"approvalId":"scope-1"}"#);
        assert!(facts(&db, now()).unwrap().is_empty());
    }

    #[test]
    fn a_resolution_for_a_different_approval_does_not_settle_this_one() {
        let db = memory_db();
        seed(&db);
        add_entry(&db, "parent", 4, "approval.requested", r#"{"title":"First"}"#);
        add_entry(&db, "parent", 6, "approval.requested", r#"{"title":"Second"}"#);
        add_entry(
            &db,
            "parent",
            7,
            "approval.resolved",
            r#"{"data":{"requestEventId":4,"decision":"accept"}}"#,
        );
        assert_eq!(keys(&facts(&db, now()).unwrap()), vec!["approval:parent:6"]);
    }

    #[test]
    fn an_approval_on_a_finished_session_is_not_offered() {
        for terminal in ["stopped", "failed", "completed", "cancelled"] {
            let db = memory_db();
            seed(&db);
            add_session(&db, "child", Some("parent"), terminal);
            add_entry(&db, "child", 4, "approval.requested", r#"{"title":"Approve command"}"#);
            assert!(
                facts(&db, now()).unwrap().is_empty(),
                "a {terminal} session has no adapter left to answer to"
            );
        }
    }

    #[test]
    fn facts_are_ordered_by_severity_then_age_then_key() {
        let db = memory_db();
        seed(&db);
        // An overdue approval and a required failed check are both blocking; the
        // check is older, so it comes first. The optional check is attention.
        add_entry(&db, "parent", 4, "approval.requested", r#"{"title":"Approve command"}"#);
        add_check(&db, "a-1", "cargo-test", true, "failed", "running", "2026-08-19T07:00:00+00:00");
        add_check(&db, "a-1", "clippy", false, "failed", "running", "2026-08-19T06:00:00+00:00");
        let later = instant("2026-08-19T11:00:00+00:00").unwrap();
        let facts = facts(&db, later).unwrap();
        assert_eq!(
            keys(&facts),
            vec![
                "completion-check:a-1:cargo-test",
                "approval:parent:4",
                "completion-check:a-1:clippy",
            ],
            "blocking before attention, and oldest first inside a severity"
        );
    }

    #[test]
    fn prompt_mutation_approval_remains_reviewable_after_origin_stops() {
        let db = memory_db();
        seed(&db);
        add_session(&db, "child", None, "stopped");
        add_entry(&db, "child", 4, "approval.requested",
            r#"{"title":"Review shared role guidance","approvalType":"prompt_mutation","approvalId":"prompt-1"}"#);
        let pending = facts(&db, now()).unwrap();
        assert_eq!(keys(&pending), vec!["approval:child:4"]);
        assert!(matches!(pending[0].action,
            wire::WorkFactAction::AnswerApproval { approval_sequence: Some(4), .. }));
        add_entry(&db, "child", 5, "approval.resolved",
            r#"{"approvalId":"prompt-1","requestEventId":4,"decision":"decline"}"#);
        assert!(facts(&db, now()).unwrap().is_empty());
    }

    #[test]
    fn the_key_tie_break_makes_the_order_total() {
        // Same severity and same instant: only the key can separate them, and it
        // always does, so the board never depends on SQLite's row order.
        let facts = finalize(vec![
            projected(sample_fact("b", wire::WorkFactSeverity::Attention, "2026-08-19T08:00:00+00:00")),
            projected(sample_fact("a", wire::WorkFactSeverity::Attention, "2026-08-19T08:00:00+00:00")),
        ]);
        assert_eq!(keys(&facts), vec!["a", "b"]);
    }

    #[test]
    fn an_unparseable_timestamp_sorts_last_rather_than_first() {
        let facts = finalize(vec![
            projected(sample_fact("unknown", wire::WorkFactSeverity::Attention, "whenever")),
            projected(sample_fact("known", wire::WorkFactSeverity::Attention, "2026-08-19T08:00:00+00:00")),
        ]);
        assert_eq!(
            keys(&facts),
            vec!["known", "unknown"],
            "an unreadable timestamp is unknown, which is not the same as oldest"
        );
    }

    #[test]
    fn two_projections_of_one_condition_collapse_to_a_single_fact() {
        let facts = finalize(vec![
            projected(sample_fact("same", wire::WorkFactSeverity::Blocking, "2026-08-19T08:00:00+00:00")),
            projected(sample_fact("same", wire::WorkFactSeverity::Blocking, "2026-08-19T09:00:00+00:00")),
            projected(sample_fact("other", wire::WorkFactSeverity::Blocking, "2026-08-19T08:30:00+00:00")),
        ]);
        assert_eq!(keys(&facts), vec!["same", "other"]);
        assert_eq!(
            facts[0].actionable_at, "2026-08-19T08:00:00+00:00",
            "the oldest of a duplicated pair survives, because it sorted first"
        );
    }

    fn sample_fact(key: &str, severity: wire::WorkFactSeverity, at: &str) -> wire::WorkFact {
        wire::WorkFact {
            kind: wire::WorkFactKind::ActionableApproval,
            dedupe_key: key.into(),
            severity,
            title: key.into(),
            detail: None,
            target: wire::WorkFactTarget::Session { session_id: "parent".into() },
            actionable_at: at.into(),
            observed_at: at.into(),
            freshness: wire::WorkFactFreshness::Live,
            action: wire::WorkFactAction::AnswerApproval {
                session_id: "parent".into(),
                approval_sequence: None,
            },
        }
    }

    // -----------------------------------------------------------------------
    // The board is store-only
    // -----------------------------------------------------------------------

    #[test]
    fn reading_the_board_starts_no_git_process() {
        let db = memory_db();
        seed(&db);
        // Point the workspace at a path that is not a repository and does not
        // exist. Anything that reached for git would fail or report
        // "unavailable"; the board instead answers from what it was told.
        db.execute("UPDATE workspaces SET path='/nonexistent/not-a-repo' WHERE id='w'", [])
            .unwrap();
        crate::work_observation::record_base_divergence(
            &db,
            "w",
            Ok(&crate::git::BaseBranchDivergence {
                base_ref: Some("origin/main".into()),
                base_commit: Some("abc".into()),
                head: Some("def".into()),
                branch: Some("bridge/task".into()),
                ahead: 0,
                behind: 40,
                ref_age_seconds: None,
                fetch_attempted: false,
                fetched: false,
                dirty: false,
                unavailable_reason: None,
            }),
            instant("2026-08-19T09:00:00+00:00").unwrap(),
        )
        .unwrap();
        add_check(&db, "a-1", "cargo-test", true, "failed", "running", "2026-08-19T08:00:00+00:00");
        add_entry(&db, "parent", 4, "approval.requested", r#"{"title":"Approve command"}"#);
        add_blocked_queue_item(&db, "q-1", "parent", "2026-08-19T08:00:00+00:00");

        let before = crate::git::git_processes_started_on_this_thread();
        let facts = facts(&db, instant("2026-08-19T09:05:00+00:00").unwrap()).unwrap();
        let board = board(&db).unwrap();
        let after = crate::git::git_processes_started_on_this_thread();

        assert_eq!(after, before, "the board read must not shell out to git");
        assert_eq!(facts.len(), 4, "and it must still project every kind while doing so");
        assert!(board.facts.is_empty(), "local facts never appear in integration activity");
    }

    #[test]
    fn the_git_process_counter_is_not_vacuous() {
        // A counter that never moves would make the test above prove nothing.
        // Measuring a real directory moves it.
        let repository = tempfile::tempdir().unwrap();
        let before = crate::git::git_processes_started_on_this_thread();
        let _ = crate::git::base_branch_divergence(repository.path(), false);
        assert!(
            crate::git::git_processes_started_on_this_thread() > before,
            "measuring divergence starts git, so a zero delta above means something"
        );
    }

    #[test]
    fn the_board_read_path_reaches_for_no_io() {
        // The projections take a `&Connection`, so they cannot reach the runtime.
        // This gate covers the other half: that nothing in this module grows a
        // subprocess, an HTTP client, or a connector call over time. Type names
        // are fine — `git::BaseBranchDivergence` is how a cached payload is
        // read — so what is banned is the call, `git::` followed by a function.
        // Only the production half: the tests below legitimately observe
        // divergence to set a fixture up, and this module's own gate literal
        // would otherwise match itself.
        let source = include_str!("work.rs");
        let source = &source[..source.find("#[cfg(test)]").expect("this module has tests")];
        for forbidden in [
            "Command::new",
            "std::process",
            "reqwest",
            "marketplace::",
            "adapters::",
            "core.adapters",
        ] {
            assert!(
                !source.contains(forbidden),
                "the board read path must not reference {forbidden}"
            );
        }
        assert!(
            source.contains("git::"),
            "this module does name a git type, so the loop below is not vacuous"
        );
        for (index, _) in source.match_indices("git::") {
            let next = source[index + "git::".len()..]
                .chars()
                .next()
                .unwrap_or(' ');
            assert!(
                next.is_ascii_uppercase(),
                "work.rs may name a git type but must not call a git function: {}",
                &source[index..(index + 48).min(source.len())]
            );
        }
    }

    #[test]
    fn reading_the_board_through_the_api_starts_no_provider() {
        // The whole way in, not just the projection: `work::board` takes a
        // `&Connection` and so has nothing to start a provider with, and this
        // pins that the one caller that *does* hold the runtime does not either.
        let fixture = tempfile::tempdir().unwrap();
        let data_dir = fixture.path();
        {
            let db = store::open(&data_dir.join("bridge.db")).unwrap();
            seed(&db);
            // Boot reconciles `working` and `waiting` sessions to `stopped`,
            // which would correctly take the approval off the board — the point
            // here is the read path, so leave the session somewhere boot keeps.
            db.execute("UPDATE sessions SET status='idle' WHERE id='parent'", []).unwrap();
            add_entry(&db, "parent", 4, "approval.requested", r#"{"title":"Approve command"}"#);
        }
        let core = std::sync::Arc::new(
            crate::BridgeCore::boot(crate::BootConfig {
                data_dir: data_dir.to_path_buf(),
                browser_extension_path: data_dir.join("no-extension"),
                events: None,
            })
            .unwrap(),
        );

        let before = crate::git::git_processes_started_on_this_thread();
        let board = crate::api::get_work_board(&core).unwrap();
        assert_eq!(crate::git::git_processes_started_on_this_thread(), before);

        assert!(board.facts.is_empty(), "local approvals stay off the integration board");
        assert!(
            core.adapters.lock().unwrap().is_empty(),
            "no adapter runtime may be started to render a board"
        );
        assert!(
            core.runtimes.lock().unwrap().is_empty(),
            "and no PTY session either"
        );
    }

    #[test]
    fn absent_settings_read_as_the_documented_defaults() {
        let db = memory_db();
        let board = board(&db).unwrap();
        assert_eq!(board.settings, default_settings());
        assert!(board.settings.briefing.is_none());
        assert_eq!(board.settings.limits.max_wall_seconds, DEFAULT_MAX_WALL_SECONDS);
        assert_eq!(board.suggestions.state, wire::WorkSuggestionsState::NotConfigured);
    }

    #[test]
    fn a_board_with_no_model_or_connector_is_still_a_board() {
        let db = memory_db();
        let board = board(&db).unwrap();
        assert!(board.tasks.is_empty(), "no briefing runner has produced tasks");
        assert!(board.latest_run.is_none());
        assert!(board.sources.is_empty());
        assert!(board.usage.is_none());
        assert!(!board.generated_at.is_empty());
    }

    #[test]
    fn the_board_projects_durable_tasks_and_reactivates_an_expired_snooze() {
        let db = memory_db();
        let safe_target = serde_json::to_string(&crate::work_connectors::EvidenceTarget::ExternalLink {
            url: "https://app.slack.com/archives/C1/p1".into(), host: "app.slack.com".into(),
        }).unwrap();
        db.execute(
            "INSERT INTO work_tasks(
                 id,fingerprint,connector_instance_id,canonical_resource_id,source_kind,
                 title,why,rank,confidence_bps,state,snoozed_until,evidence_target,
                 ephemeral,created_at,updated_at,source_activity_at)
             VALUES('task-1','v1:one','slack-1','slack:slack-1:1','slack.message',
                    'Reply','Asked twice',1,8200,'snoozed','2026-08-19T08:00:00+00:00',?1,0,
                    '2026-08-19T07:00:00+00:00','2026-08-19T07:00:00+00:00','2026-08-19T07:00:00+00:00')",
            rusqlite::params![safe_target],
        ).unwrap();

        let tasks = suggested_tasks(&db, now()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "task-1");
        assert_eq!(tasks[0].fingerprint.as_deref(), Some("v1:one"));
        assert_eq!(tasks[0].state, wire::WorkTaskState::Active);
        assert!(tasks[0].snoozed_until.is_none());
        assert_eq!(tasks[0].evidence_target, Some(wire::WorkEvidenceTarget::ExternalLink {
            url: "https://app.slack.com/archives/C1/p1".into(), host: "app.slack.com".into(),
        }));
        let stored: (String, Option<String>) = db.query_row(
            "SELECT state,snoozed_until FROM work_tasks WHERE id='task-1'", [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(stored, ("active".into(), None));
    }

    #[test]
    fn the_board_hides_legacy_tasks_even_when_pinned() {
        let db = memory_db();
        for (id, state, pinned) in [("hidden", "done", 1), ("pinned", "stale", 1)] {
            db.execute(
                "INSERT INTO work_tasks(
                     id,connector_instance_id,source_kind,title,why,rank,confidence_bps,
                     state,pinned,created_at,updated_at)
                 VALUES(?1,'slack-1','slack.message','Reply','Asked twice',1,8200,?2,?3,'now','now')",
                rusqlite::params![id, state, pinned],
            ).unwrap();
        }
        let tasks = suggested_tasks(&db, now()).unwrap();
        assert!(tasks.is_empty());
    }

    #[test]
    fn configured_settings_are_read_back_verbatim() {
        let db = memory_db();
        store_settings(
            &db,
            r#"{"briefing":{"harness":"claude","model":"claude-opus-5","effort":"high"},
                "enabledConnectorInstances":["github:acme"],"refreshOnFocus":true,
                "refreshIntervalMinutes":30,"cooldownMinutes":20,
                "limits":{"maxWallSeconds":300,"maxTurns":6,"maxToolCalls":12,
                          "maxOutputTokens":null,"costCeilingMicrousd":null}}"#,
        );
        let board = board(&db).unwrap();
        let briefing = board.settings.briefing.as_ref().expect("briefing profile");
        assert_eq!(briefing.harness.as_str(), "claude");
        assert_eq!(briefing.model, "claude-opus-5");
        assert_eq!(board.settings.enabled_connector_instances, vec!["github:acme".to_owned()]);
        assert_eq!(board.settings.cooldown_minutes, 20);
        assert_eq!(board.settings.limits.max_turns, 6);
    }

    #[test]
    fn settings_that_cannot_be_read_degrade_instead_of_failing_the_board() {
        let db = memory_db();
        // An unknown field is exactly what a rolled-back binary would meet.
        store_settings(
            &db,
            r#"{"briefing":null,"enabledConnectorInstances":[],"refreshOnFocus":false,
                "refreshIntervalMinutes":null,"cooldownMinutes":15,
                "limits":{"maxWallSeconds":600,"maxTurns":12,"maxToolCalls":24,
                          "maxOutputTokens":null,"costCeilingMicrousd":null},
                "writeConnectorTools":true}"#,
        );
        let board = board(&db).unwrap();
        assert_eq!(board.settings, default_settings(), "defaults, not a half-read payload");
        assert_eq!(board.suggestions.state, wire::WorkSuggestionsState::Degraded);
        assert!(board
            .suggestions
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("could not be read")));
    }
    #[test]
    fn integration_window_filters_before_limiting_and_uses_source_time_only() {
        let db = memory_db();
        let end = instant("2026-09-10T12:00:00Z").unwrap();
        let insert = |id: &str, activity: Option<&str>, state: &str, source: &str, rank: i64| {
            db.execute("INSERT INTO work_tasks(id,connector_instance_id,source_kind,title,why,rank,confidence_bps,state,pinned,created_at,updated_at,evidence_observed_at,source_activity_at)
                VALUES(?1,'integration',?2,?1,'update',?3,8000,?4,1,?5,?5,?5,?6)",
                rusqlite::params![id, source, rank, state, end.to_rfc3339(), activity]).unwrap();
        };
        for index in 0..110 {
            insert(&format!("stale-{index}"), Some("2026-09-10T11:59:00Z"), "stale", "slack.message", 1);
            insert(&format!("old-{index}"), Some("2020-01-01T00:00:00Z"), "active", "slack.message", 1);
        }
        for (id, at) in [("undated", None), ("invalid", Some("garbage")), ("expired", Some("2026-09-09T11:59:59Z")), ("future", Some("2026-09-10T12:00:01Z"))] {
            insert(id, at, "active", "github.item", 1);
        }
        insert("local", Some("2026-09-10T12:00:00Z"), "active", "bridge.check", 1);
        insert("boundary", Some("2026-09-09T12:00:00Z"), "active", "slack.message", 1);
        insert("newest", Some("2026-09-10T12:00:00Z"), "active", "github.item", 1000);
        let tasks = suggested_tasks(&db, end).unwrap();
        assert_eq!(tasks.iter().map(|task| task.id.as_str()).collect::<Vec<_>>(), ["newest", "boundary"]);
        assert_eq!(tasks[1].source_activity_at.as_deref(), Some("2026-09-09T12:00:00Z"));
    }

}

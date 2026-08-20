use crate::{model::*, BridgeError};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
use uuid::Uuid;

const LATEST_SCHEMA_VERSION: i64 = 30;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetrySpan {
    pub span_id: String,
    pub trace_id: String,
    pub name: String,
    pub attributes: String,
    pub started_at: String,
    pub ended_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistorySnapshotManifest {
    pub schema_version: u32,
    pub database_file: String,
    pub sha256: String,
    pub created_at: String,
}

pub fn open(path: &Path) -> Result<Connection, BridgeError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut connection = Connection::open(path)?;
    // Migrations run with foreign keys disabled so table rebuilds (which drop and
    // recreate parent tables) don't trip referential checks; re-enabled after.
    connection.execute_batch("PRAGMA foreign_keys=OFF;")?;
    run_migrations(&mut connection, path)?;
    connection.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
    )?;
    // Chats created before titles existed still read "Orchestrator"; name them
    // from what they already contain. Local-only, so opening stays cheap.
    let _ = crate::session_titles::backfill_from_messages(&connection);
    let now = Utc::now().to_rfc3339();
    connection.execute(
        "INSERT OR IGNORE INTO session_heads(session_id,native_provider_session_id,restoration_mode,resume_eligibility,updated_at)
         SELECT id,provider_session_id,'fresh',CASE WHEN provider_session_id IS NOT NULL THEN 'native' ELSE 'fresh' END,?1
         FROM sessions WHERE status IN ('working','waiting')",
        params![now],
    )?;
    connection.execute(
        "UPDATE session_heads
         SET native_provider_session_id=COALESCE(native_provider_session_id,(SELECT provider_session_id FROM sessions WHERE sessions.id=session_heads.session_id)),
             resume_eligibility=CASE
                 WHEN COALESCE(native_provider_session_id,(SELECT provider_session_id FROM sessions WHERE sessions.id=session_heads.session_id)) IS NOT NULL THEN 'native'
                 WHEN active_entry_id IS NOT NULL OR latest_checkpoint_entry_id IS NOT NULL THEN 'checkpoint_restored'
                 ELSE 'fresh'
             END,
             updated_at=?1
         WHERE session_id IN (SELECT id FROM sessions WHERE status IN ('working','waiting'))",
        params![now],
    )?;
    connection.execute(
        "UPDATE worker_leases SET lease_status='expired',updated_at=?1
         WHERE lease_status IN ('active','warm') AND session_id IN (
            SELECT session_id FROM worker_runtime
            WHERE lifecycle_state IN ('starting','working','waiting','warm','checkpointing','resuming','restored','failed')
         )",
        params![now],
    )?;
    connection.execute(
        "UPDATE sessions SET status='stopped', ended_at=?1 WHERE status IN ('working','waiting')",
        params![now],
    )?;
    connection.execute(
        "UPDATE workspaces SET status='stopped' WHERE status IN ('working','waiting')",
        [],
    )?;
    Ok(connection)
}

pub fn open_telemetry(path: &Path) -> Result<Connection, BridgeError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let connection = Connection::open(path)?;
    connection.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA busy_timeout=50;
         CREATE TABLE IF NOT EXISTS telemetry_spans (
            span_id TEXT PRIMARY KEY,
            trace_id TEXT NOT NULL,
            parent_span_id TEXT,
            name TEXT NOT NULL,
            attributes TEXT NOT NULL,
            started_at TEXT NOT NULL,
            ended_at TEXT
         );
         CREATE INDEX IF NOT EXISTS idx_telemetry_trace ON telemetry_spans(trace_id,started_at);",
    )?;
    Ok(connection)
}

pub fn telemetry_span(
    trace_id: &str,
    session_id: &str,
    adapter_id: &str,
    event: &crate::agent::NormalizedEvent,
    occurred_at: &str,
) -> TelemetrySpan {
    TelemetrySpan {
        span_id: Uuid::new_v4().simple().to_string(),
        trace_id: trace_id.to_owned(),
        name: format!("gen_ai.{}", event.kind.replace('.', "_")),
        attributes: serde_json::json!({
            "gen_ai.operation.name": event.kind,
            "gen_ai.provider.name": adapter_id,
            "gen_ai.conversation.id": session_id,
        })
        .to_string(),
        started_at: occurred_at.to_owned(),
        ended_at: occurred_at.to_owned(),
    }
}

pub fn append_telemetry_batch(
    db: &Connection,
    spans: &[TelemetrySpan],
) -> Result<usize, BridgeError> {
    if spans.is_empty() {
        return Ok(0);
    }
    let transaction = db.unchecked_transaction()?;
    for span in spans {
        transaction.execute(
            "INSERT OR IGNORE INTO telemetry_spans(span_id,trace_id,name,attributes,started_at,ended_at)
             VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                span.span_id,
                span.trace_id,
                span.name,
                span.attributes,
                span.started_at,
                span.ended_at,
            ],
        )?;
    }
    transaction.commit()?;
    Ok(spans.len())
}

pub fn export_history_snapshot(
    db: &Connection,
    snapshot_dir: &Path,
) -> Result<(PathBuf, PathBuf), BridgeError> {
    std::fs::create_dir_all(snapshot_dir)?;
    let id = format!(
        "{}-{}",
        Utc::now().format("%Y%m%dT%H%M%S%fZ"),
        Uuid::new_v4().simple()
    );
    let database_path = snapshot_dir.join(format!("bridge-history-{id}.sqlite"));
    let escaped = database_path.to_string_lossy().replace('\'', "''");
    db.execute_batch(&format!("VACUUM INTO '{escaped}'"))?;
    let bytes = std::fs::read(&database_path)?;
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    let manifest = HistorySnapshotManifest {
        schema_version: 1,
        database_file: database_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned(),
        sha256,
        created_at: Utc::now().to_rfc3339(),
    };
    let manifest_path = snapshot_dir.join(format!("bridge-history-{id}.manifest.json"));
    let pending_manifest = snapshot_dir.join(format!(".{id}.manifest.tmp"));
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    std::fs::write(&pending_manifest, manifest_bytes)?;
    std::fs::rename(&pending_manifest, &manifest_path)?;
    Ok((database_path, manifest_path))
}

pub fn verify_history_snapshot(
    database_path: &Path,
    manifest_path: &Path,
) -> Result<bool, BridgeError> {
    let manifest: HistorySnapshotManifest = serde_json::from_slice(&std::fs::read(manifest_path)?)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    if manifest.schema_version != 1
        || database_path.file_name().and_then(|name| name.to_str())
            != Some(manifest.database_file.as_str())
    {
        return Ok(false);
    }
    let actual = format!("{:x}", Sha256::digest(std::fs::read(database_path)?));
    Ok(actual == manifest.sha256)
}

fn run_migrations(connection: &mut Connection, path: &Path) -> Result<(), BridgeError> {
    let current = current_schema_version(connection)?;
    if current >= LATEST_SCHEMA_VERSION {
        return Ok(());
    }

    if has_user_schema(connection)? && path != Path::new(":memory:") {
        let (busy, _, _): (i64, i64, i64) =
            connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;
        if busy != 0 {
            return Err(BridgeError::Invalid(
                "database WAL is busy; refusing to create an incomplete migration backup".into(),
            ));
        }
        backup_database(path)?;
    }

    for version in (current + 1)..=LATEST_SCHEMA_VERSION {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        match version {
            1 => migration_1_current_schema(&transaction)?,
            2 => migration_2_session_forest(&transaction)?,
            3 => migration_3_capability_tiers(&transaction)?,
            4 => migration_4_resume_eligibility(&transaction)?,
            5 => migration_5_durable_worker_pool(&transaction)?,
            6 => migration_6_remove_legacy_agent_events(&transaction)?,
            7 => migration_7_optional_repo_and_direct_chats(&transaction)?,
            8 => migration_8_reliability_primitives(&transaction)?,
            9 => migration_9_semantic_event_version(&transaction)?,
            10 => migration_10_continuation_fidelity(&transaction)?,
            11 => migration_11_human_blocked_queue(&transaction)?,
            12 => migration_12_adapter_process_claims(&transaction)?,
            13 => migration_13_learning_router(&transaction)?,
            14 => migration_14_completion_proof(&transaction)?,
            15 => migration_15_role_profiles_and_learning_jobs(&transaction)?,
            16 => migration_16_complete_role_profile_schema(&transaction)?,
            17 => migration_17_configuration_entries(&transaction)?,
            18 => migration_18_prompt_cache_telemetry(&transaction)?,
            19 => migration_19_repair_learning_router_schema(&transaction)?,
            20 => migration_20_repair_legacy_learning_constraints(&transaction)?,
            21 => migration_21_approval_deadlines_and_worktree_adoption(&transaction)?,
            22 => migration_22_session_backend_binding(&transaction)?,
            23 => migration_23_session_title_source(&transaction)?,
            24 => migration_24_work_board(&transaction)?,
            25 => migration_25_ephemeral_work_evidence(&transaction)?,
            26 => migration_26_briefing_run_leases(&transaction)?,
            27 => migration_27_queued_session_input(&transaction)?,
            28 => migration_28_evidence_based_retries(&transaction)?,
            29 => migration_29_learning_scope(&transaction)?,
            30 => migration_30_session_entry_fts(&transaction)?,
            _ => {
                return Err(BridgeError::Invalid(format!(
                    "unknown schema migration {version}"
                )))
            }
        }
        transaction.execute(
            "INSERT INTO schema_version(version, applied_at) VALUES(?1, ?2)",
            params![version, Utc::now().to_rfc3339()],
        )?;
        transaction.commit()?;
    }
    Ok(())
}

fn migration_17_configuration_entries(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE configuration_entries (
            kind TEXT NOT NULL,
            id TEXT NOT NULL,
            payload TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY(kind,id)
        );
        CREATE INDEX idx_configuration_entries_kind ON configuration_entries(kind,updated_at);",
    )?;
    Ok(())
}

fn migration_18_prompt_cache_telemetry(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    add_column_if_missing(
        transaction,
        "usage_ledger",
        "uncached_input_tokens",
        "INTEGER",
    )?;
    add_column_if_missing(transaction, "usage_ledger", "stable_prefix_id", "TEXT")?;
    add_column_if_missing(transaction, "usage_ledger", "stable_prefix_hash", "TEXT")?;
    add_column_if_missing(
        transaction,
        "usage_ledger",
        "prompt_schema_version",
        "INTEGER",
    )?;
    add_column_if_missing(
        transaction,
        "usage_ledger",
        "prefix_token_estimate",
        "INTEGER",
    )?;
    add_column_if_missing(transaction, "usage_ledger", "harness", "TEXT")?;
    add_column_if_missing(transaction, "usage_ledger", "model", "TEXT")?;
    add_column_if_missing(transaction, "usage_ledger", "role", "TEXT")?;
    add_column_if_missing(transaction, "usage_ledger", "task_family", "TEXT")?;
    add_column_if_missing(transaction, "usage_ledger", "restoration_mode", "TEXT")?;
    add_column_if_missing(transaction, "usage_ledger", "cross_harness_reuse", "TEXT")?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS prompt_compilations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            turn_id TEXT,
            prefix_id TEXT NOT NULL,
            prefix_hash TEXT NOT NULL,
            schema_version INTEGER NOT NULL,
            prefix_bytes INTEGER NOT NULL,
            prefix_token_estimate INTEGER NOT NULL,
            harness TEXT NOT NULL,
            model TEXT,
            role TEXT NOT NULL,
            task_family TEXT NOT NULL,
            restoration_mode TEXT NOT NULL,
            cross_harness_reuse TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_prompt_compilations_session
            ON prompt_compilations(session_id,id DESC);
        CREATE INDEX IF NOT EXISTS idx_prompt_compilations_prefix
            ON prompt_compilations(harness,prefix_hash,id DESC);",
    )?;
    add_column_if_missing(transaction, "prompt_compilations", "turn_id", "TEXT")?;
    transaction.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_prompt_compilations_turn
            ON prompt_compilations(session_id,turn_id,id DESC);",
    )?;
    Ok(())
}

fn migration_19_repair_learning_router_schema(
    transaction: &Transaction<'_>,
) -> Result<(), BridgeError> {
    // Migration 15 changed while several feature branches were shipping with
    // the same schema version. Databases that recorded the earlier v15 shape
    // never received the later router columns, so worker routing failed before
    // a child session could be created. Replaying the idempotent migration
    // repairs every affected table instead of only the first missing column.
    add_column_if_missing(
        transaction,
        "learning_jobs",
        "run_budget_tokens",
        "INTEGER NOT NULL DEFAULT 50000",
    )?;
    add_column_if_missing(transaction, "learning_triggers", "auth_digest", "TEXT")?;
    add_column_if_missing(transaction, "learning_triggers", "expires_at", "TEXT")?;
    add_column_if_missing(
        transaction,
        "learning_triggers",
        "experimental",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        transaction,
        "learning_triggers",
        "updated_at",
        "TEXT NOT NULL DEFAULT '1970-01-01T00:00:00Z'",
    )?;
    add_column_if_missing(transaction, "learning_job_runs", "lease_owner", "TEXT")?;
    add_column_if_missing(transaction, "learning_job_runs", "lease_expires_at", "TEXT")?;
    add_column_if_missing(
        transaction,
        "learning_job_runs",
        "snapshot_frozen_at",
        "TEXT NOT NULL DEFAULT '1970-01-01T00:00:00Z'",
    )?;
    add_column_if_missing(
        transaction,
        "learning_job_runs",
        "evaluated_spend_microusd",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        transaction,
        "learning_job_runs",
        "evaluated_tokens",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(transaction, "learning_job_runs", "replay_passed", "INTEGER")?;
    add_column_if_missing(
        transaction,
        "learning_job_runs",
        "promotion_status",
        "TEXT NOT NULL DEFAULT 'not_requested'",
    )?;
    add_column_if_missing(
        transaction,
        "routing_evaluations",
        "learning_run_id",
        "TEXT REFERENCES learning_job_runs(id) ON DELETE SET NULL",
    )?;
    add_column_if_missing(
        transaction,
        "routing_evaluations",
        "decision_id",
        "TEXT REFERENCES router_decisions(id) ON DELETE CASCADE",
    )?;
    add_column_if_missing(
        transaction,
        "routing_evaluations",
        "bounded_metrics",
        "TEXT NOT NULL DEFAULT '{}'",
    )?;
    add_column_if_missing(
        transaction,
        "routing_evaluations",
        "status",
        "TEXT NOT NULL DEFAULT 'completed'",
    )?;
    add_column_if_missing(
        transaction,
        "routing_policies",
        "rollback_of",
        "INTEGER REFERENCES routing_policies(version)",
    )?;
    add_column_if_missing(transaction, "routing_policies", "replay_report", "TEXT")?;
    add_column_if_missing(transaction, "routing_policies", "promoted_at", "TEXT")?;
    add_column_if_missing(
        transaction,
        "routing_policies",
        "activation_boundary",
        "INTEGER",
    )?;
    add_column_if_missing(
        transaction,
        "learning_trigger_events",
        "registration_id",
        "TEXT",
    )?;
    add_column_if_missing(transaction, "learning_trigger_events", "reason", "TEXT")?;
    migration_15_role_profiles_and_learning_jobs(transaction)?;
    if column_exists(transaction, "routing_evaluations", "run_id")? {
        transaction.execute(
            "UPDATE routing_evaluations SET learning_run_id=run_id WHERE learning_run_id IS NULL",
            [],
        )?;
    }
    migration_16_complete_role_profile_schema(transaction)
}

fn migration_20_repair_legacy_learning_constraints(
    transaction: &Transaction<'_>,
) -> Result<(), BridgeError> {
    // The first migration-15 shape used a required `run_id` column. Adding the
    // current columns did not relax that constraint, so inserts that correctly
    // omit the legacy field still failed after v19. Preserve its data, then
    // remove it now that `learning_run_id` is canonical.
    if table_exists(transaction, "routing_evaluations")?
        && column_exists(transaction, "routing_evaluations", "run_id")?
    {
        add_column_if_missing(
            transaction,
            "routing_evaluations",
            "learning_run_id",
            "TEXT REFERENCES learning_job_runs(id) ON DELETE SET NULL",
        )?;
        transaction.execute(
            "UPDATE routing_evaluations SET learning_run_id=run_id WHERE learning_run_id IS NULL",
            [],
        )?;
        transaction.execute_batch("ALTER TABLE routing_evaluations DROP COLUMN run_id;")?;
    }

    // Rebuild instead of ALTERing because the legacy column was NOT NULL and
    // used ON DELETE CASCADE; SQLite cannot relax either property in place.
    if table_exists(transaction, "learning_trigger_events")? {
        transaction.execute_batch(
            "CREATE TABLE learning_trigger_events_v20 (
                id TEXT PRIMARY KEY,
                run_id TEXT REFERENCES learning_job_runs(id) ON DELETE SET NULL,
                trigger_kind TEXT NOT NULL,
                registration_id TEXT,
                result TEXT NOT NULL,
                reason TEXT,
                created_at TEXT NOT NULL
            );
            INSERT INTO learning_trigger_events_v20(id,run_id,trigger_kind,registration_id,result,reason,created_at)
                SELECT id,run_id,trigger_kind,registration_id,result,reason,created_at
                FROM learning_trigger_events;
            DROP TABLE learning_trigger_events;
            ALTER TABLE learning_trigger_events_v20 RENAME TO learning_trigger_events;",
        )?;
    }

    // Index names, unlike definitions, satisfy IF NOT EXISTS. Recreate this
    // invariant explicitly so an active policy and a canary cannot coexist.
    if table_exists(transaction, "routing_policies")? {
        transaction.execute_batch(
            "DROP INDEX IF EXISTS idx_routing_policy_active;
             CREATE UNIQUE INDEX idx_routing_policy_active
                ON routing_policies((1)) WHERE status IN ('active','canary');",
        )?;
    }

    add_column_if_missing(transaction, "worker_runtime", "last_activity_at", "TEXT")
}

fn migration_21_approval_deadlines_and_worktree_adoption(
    transaction: &Transaction<'_>,
) -> Result<(), BridgeError> {
    // `waiting` workers were excluded from every watchdog, so an unanswered
    // in-session approval left the worker pending forever. Stamping the entry
    // time gives the approval deadline something durable to measure.
    add_column_if_missing(transaction, "worker_runtime", "waiting_since", "TEXT")?;
    add_column_if_missing(transaction, "worker_runtime", "waiting_reason", "TEXT")?;
    // Completion evidence used to be a bare HEAD string, which cannot say what
    // the change is relative to or which worker revision produced it.
    add_column_if_missing(transaction, "eval_attempts", "base_ref", "TEXT")?;
    add_column_if_missing(transaction, "eval_attempts", "base_commit", "TEXT")?;
    add_column_if_missing(transaction, "eval_attempts", "worker_branch", "TEXT")?;
    add_column_if_missing(transaction, "eval_attempts", "worker_session_id", "TEXT")?;
    // Why an attempt ended, when it ended for a reason other than its checks.
    // A gate that could not be built must keep failing closed; a gate that ran
    // out of time must release the parent so it can report.
    add_column_if_missing(transaction, "eval_attempts", "escalation", "TEXT")?;
    // Verified work that lives only in a child worktree must not silently
    // disappear: it needs a durable adoption state that survives restart.
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS worker_worktree_adoptions (
            session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
            parent_session_id TEXT NOT NULL,
            workspace_id TEXT NOT NULL,
            worktree_path TEXT NOT NULL,
            worktree_branch TEXT NOT NULL,
            task_worktree_path TEXT NOT NULL,
            state TEXT NOT NULL,
            head TEXT,
            base_commit TEXT,
            base_branch TEXT,
            baseline_dirty_paths TEXT NOT NULL DEFAULT '[]',
            changed_paths TEXT NOT NULL DEFAULT '[]',
            diffstat TEXT,
            dirty INTEGER NOT NULL DEFAULT 0,
            detail TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_worker_worktree_adoptions_parent
            ON worker_worktree_adoptions(parent_session_id,state);",
    )?;
    Ok(())
}

/// Which backend actually served a session, and the authorization to change it.
///
/// `sessions.harness` says which agent a user picked. It has never said which
/// implementation ran, because until the marketplace there was only ever one —
/// so a session resumed after an install or a backend swap had no way to know it
/// had moved. These three columns are that missing provenance.
///
/// All nullable, and no backfill. A row written before this migration is
/// genuinely unbound rather than bound to a guess: inferring a backend for it
/// would be inventing history, and the read path treats null as "not recorded"
/// and binds it on its next successful start.
/// Records where a session's title came from, so a heading Bridge derived from the
/// first message can later be replaced by the one the harness writes, while a
/// title the user or the provider chose is never overwritten.
fn migration_23_session_title_source(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    let has_column = transaction
        .prepare("SELECT 1 FROM pragma_table_info('sessions') WHERE name='title_source'")?
        .exists([])?;
    if !has_column {
        transaction.execute_batch("ALTER TABLE sessions ADD COLUMN title_source TEXT;")?;
    }
    // Titles that predate this column were set by the user at creation, so they
    // stay untouched: an absent source is read as "not ours to replace".
    Ok(())
}

fn migration_22_session_backend_binding(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    add_column_if_missing(transaction, "sessions", "backend_id", "TEXT")?;
    add_column_if_missing(transaction, "sessions", "backend_version", "TEXT")?;
    add_column_if_missing(transaction, "sessions", "backend_installation_id", "TEXT")?;
    // A backend change is refused until it is authorized for that exact
    // transition. One pending authorization per session, consumed when it is
    // used, so it cannot be spent twice or generalize to a later change.
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS backend_change_authorizations (
            session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
            from_backend TEXT NOT NULL,
            to_backend TEXT NOT NULL,
            to_version TEXT,
            authorized_at TEXT NOT NULL
        );",
    )?;
    Ok(())
}

fn current_schema_version(connection: &Connection) -> Result<i64, BridgeError> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_version')",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(0);
    }
    Ok(connection.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
        [],
        |row| row.get(0),
    )?)
}

fn has_user_schema(connection: &Connection) -> Result<bool, BridgeError> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name != 'schema_version')",
        [],
        |row| row.get(0),
    )?)
}

fn backup_database(path: &Path) -> Result<PathBuf, BridgeError> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("bridge.db");
    let suffix = Utc::now().format("%Y%m%dT%H%M%S%fZ");
    let backup = path.with_file_name(format!("{file_name}.backup-{suffix}"));
    std::fs::copy(path, &backup)?;
    Ok(backup)
}

/// The Work board's storage. Five tables, no changes to existing ones: a
/// database this migration has touched stays readable by the previous binary
/// apart from tables it never looks at.
///
/// `work_fact_cache` is the only one the offline board reads. The other four
/// exist so the briefing slices have somewhere to land without a second
/// migration, and so the constraints that keep a board idempotent
/// (`(run_id, evidence_ref)`, `(run_id, connector_instance_id)`, the task
/// fingerprint) are declared once, by the schema, rather than by whichever
/// writer remembers.
///
/// Nothing here holds a raw connector payload or a credential: provenance is
/// kept as digests and Bridge-derived identity, and the hidden briefing session
/// remains the diagnostic transcript.
fn migration_24_work_board(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        // `trigger_kind`, not `trigger`: TRIGGER is a SQLite keyword and a
        // column that needs quoting to be read is a column that will one day be
        // read unquoted.
        "CREATE TABLE IF NOT EXISTS work_brief_runs (
            id TEXT PRIMARY KEY,
            trigger_kind TEXT NOT NULL,
            status TEXT NOT NULL,
            profile_reference TEXT,
            session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
            max_wall_seconds INTEGER NOT NULL,
            max_turns INTEGER NOT NULL,
            max_tool_calls INTEGER NOT NULL,
            max_output_tokens INTEGER,
            cost_ceiling_microusd INTEGER,
            output_digest TEXT,
            failure_code TEXT,
            failure_detail TEXT,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cached_input_tokens INTEGER NOT NULL DEFAULT 0,
            cost_microusd INTEGER,
            tool_calls INTEGER NOT NULL DEFAULT 0,
            turns INTEGER NOT NULL DEFAULT 0,
            idempotency_key TEXT,
            started_at TEXT NOT NULL,
            completed_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_work_brief_runs_status ON work_brief_runs(status,started_at);
        -- Collapses a focus/cadence/manual race before a provider starts. Partial
        -- so runs that predate an idempotency key do not all collide on NULL.
        CREATE UNIQUE INDEX IF NOT EXISTS idx_work_brief_runs_idempotency
            ON work_brief_runs(idempotency_key) WHERE idempotency_key IS NOT NULL;
        CREATE TABLE IF NOT EXISTS work_brief_sources (
            run_id TEXT NOT NULL REFERENCES work_brief_runs(id) ON DELETE CASCADE,
            connector_instance_id TEXT NOT NULL,
            connector_family TEXT NOT NULL,
            status TEXT NOT NULL,
            detail TEXT,
            observed_at TEXT,
            UNIQUE(run_id,connector_instance_id)
        );
        CREATE INDEX IF NOT EXISTS idx_work_brief_sources_run ON work_brief_sources(run_id,status);
        CREATE TABLE IF NOT EXISTS work_evidence (
            run_id TEXT NOT NULL REFERENCES work_brief_runs(id) ON DELETE CASCADE,
            evidence_ref TEXT NOT NULL,
            tool_call_id TEXT NOT NULL,
            connector_instance_id TEXT NOT NULL,
            canonical_resource_id TEXT NOT NULL,
            source_kind TEXT NOT NULL,
            -- A serialized Bridge-derived target, never a model-authored URL.
            target TEXT,
            tool_definition_digest TEXT NOT NULL,
            result_digest TEXT NOT NULL,
            succeeded INTEGER NOT NULL DEFAULT 0,
            observed_at TEXT NOT NULL,
            UNIQUE(run_id,evidence_ref)
        );
        CREATE INDEX IF NOT EXISTS idx_work_evidence_resource
            ON work_evidence(connector_instance_id,canonical_resource_id);
        CREATE TABLE IF NOT EXISTS work_tasks (
            id TEXT PRIMARY KEY,
            -- NULL for an ephemeral task: one Bridge could not give a canonical
            -- identity. SQLite counts NULLs as distinct in a unique index, so
            -- several ephemeral tasks coexist while two identified tasks can
            -- never share a fingerprint.
            fingerprint TEXT,
            connector_instance_id TEXT NOT NULL,
            canonical_resource_id TEXT,
            source_kind TEXT NOT NULL,
            title TEXT NOT NULL,
            why TEXT NOT NULL,
            rank INTEGER NOT NULL,
            confidence_bps INTEGER NOT NULL,
            state TEXT NOT NULL DEFAULT 'active',
            pinned INTEGER NOT NULL DEFAULT 0,
            snoozed_until TEXT,
            evidence_digest TEXT,
            evidence_target TEXT,
            evidence_observed_at TEXT,
            -- Consecutive *successful* source-scoped misses. A connector failure
            -- never increments it, which is why it is stored rather than derived.
            miss_count INTEGER NOT NULL DEFAULT 0,
            ephemeral INTEGER NOT NULL DEFAULT 0,
            workspace_id TEXT REFERENCES workspaces(id) ON DELETE SET NULL,
            first_run_id TEXT REFERENCES work_brief_runs(id) ON DELETE SET NULL,
            last_run_id TEXT REFERENCES work_brief_runs(id) ON DELETE SET NULL,
            resolution TEXT,
            resolved_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE(fingerprint)
        );
        CREATE INDEX IF NOT EXISTS idx_work_tasks_board ON work_tasks(state,pinned,rank);
        CREATE INDEX IF NOT EXISTS idx_work_tasks_source
            ON work_tasks(connector_instance_id,canonical_resource_id);
        -- Snapshots of facts that cannot be observed on a store-only read path.
        -- `cache_key` is kind-defined (a workspace id for base divergence), so it
        -- carries no foreign key; every projection joins the entity it names, and
        -- a row whose entity is gone simply stops projecting.
        CREATE TABLE IF NOT EXISTS work_fact_cache (
            kind TEXT NOT NULL,
            cache_key TEXT NOT NULL,
            status TEXT NOT NULL,
            payload TEXT,
            detail TEXT,
            observed_at TEXT NOT NULL,
            PRIMARY KEY(kind,cache_key)
        );
        CREATE INDEX IF NOT EXISTS idx_work_fact_cache_observed ON work_fact_cache(kind,observed_at);",
    )?;
    Ok(())
}

/// Successful connector results without a stable provider id remain valid run-scoped
/// evidence. Their tasks are ephemeral and are never deduplicated across runs.
fn migration_25_ephemeral_work_evidence(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "DROP INDEX IF EXISTS idx_work_evidence_resource;
         ALTER TABLE work_evidence RENAME TO work_evidence_v24;
         CREATE TABLE work_evidence (
            run_id TEXT NOT NULL REFERENCES work_brief_runs(id) ON DELETE CASCADE,
            evidence_ref TEXT NOT NULL,
            tool_call_id TEXT NOT NULL,
            connector_instance_id TEXT NOT NULL,
            canonical_resource_id TEXT,
            source_kind TEXT NOT NULL,
            target TEXT,
            tool_definition_digest TEXT NOT NULL,
            result_digest TEXT NOT NULL,
            succeeded INTEGER NOT NULL DEFAULT 0,
            observed_at TEXT NOT NULL,
            UNIQUE(run_id,evidence_ref)
         );
         INSERT INTO work_evidence(
            run_id,evidence_ref,tool_call_id,connector_instance_id,canonical_resource_id,
            source_kind,target,tool_definition_digest,result_digest,succeeded,observed_at)
         SELECT run_id,evidence_ref,tool_call_id,connector_instance_id,canonical_resource_id,
            source_kind,target,tool_definition_digest,result_digest,succeeded,observed_at
         FROM work_evidence_v24;
         DROP TABLE work_evidence_v24;
         CREATE INDEX idx_work_evidence_resource
            ON work_evidence(connector_instance_id,canonical_resource_id);",
    )?;
    Ok(())
}

/// The durable lease that makes racing briefing triggers safe: one active run,
/// heartbeated by its owner, reclaimable by compare-and-swap once the lease
/// expires, and a cancellation flag the run loop polls. Columns rather than a
/// new table because a lease without a run is meaningless — it is the run row
/// that is leased.
fn migration_26_briefing_run_leases(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    // Guarded per column: the repair path replays migrations over a database
    // whose tables may already carry them, and a blind ALTER would refuse the
    // whole replay over a column that is exactly what it should be.
    for (column, definition) in [
        ("lease_owner", "TEXT"),
        ("lease_expires_at", "TEXT"),
        ("cancellation_requested", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        if !column_exists(transaction, "work_brief_runs", column)? {
            transaction.execute_batch(&format!(
                "ALTER TABLE work_brief_runs ADD COLUMN {column} {definition};"
            ))?;
        }
    }
    Ok(())
}

/// Scope learned routing policies to a workspace. Existing rows become
/// `legacy:global`, which live routing never selects. The unique live-policy
/// index stays "one active or canary", now per scope rather than globally —
/// the previous constant-expression unique index already made those two
/// statuses mutually exclusive, so the backfill cannot collide.
fn migration_29_learning_scope(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    add_column_if_missing(
        transaction,
        "routing_policies",
        "learning_scope",
        "TEXT NOT NULL DEFAULT 'legacy:global'",
    )?;
    add_column_if_missing(
        transaction,
        "learning_job_runs",
        "learning_scope",
        "TEXT NOT NULL DEFAULT 'legacy:global'",
    )?;
    add_column_if_missing(
        transaction,
        "routing_policy_promotions",
        "learning_scope",
        "TEXT NOT NULL DEFAULT 'legacy:global'",
    )?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS learning_scope_cursors (
            learning_scope TEXT PRIMARY KEY,
            last_evidence_boundary INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_router_decisions_workspace_family
            ON router_decisions(workspace_id,task_family,created_at);
        DROP INDEX IF EXISTS idx_routing_policy_active;
        CREATE UNIQUE INDEX idx_routing_policy_active
            ON routing_policies(learning_scope) WHERE status IN ('active','canary');",
    )?;
    Ok(())
}

/// The durable home for user input submitted while a turn was already running.
///
/// A follow-up the user typed must not live only in a UI state hook: a reconnect
/// or a daemon restart would lose it, and an in-memory queue drained twice would
/// deliver it twice. `state` is the exactly-once guard — delivery claims a row
/// with a compare-and-swap out of `queued`, so two concurrent drains cannot both
/// win it.
fn migration_27_queued_session_input(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS queued_session_input (
            sequence INTEGER PRIMARY KEY AUTOINCREMENT,
            id TEXT NOT NULL UNIQUE,
            session_id TEXT NOT NULL,
            provider_text TEXT NOT NULL,
            display_text TEXT NOT NULL,
            state TEXT NOT NULL CHECK(state IN ('queued','claiming','delivered','abandoned')),
            created_at TEXT NOT NULL,
            delivered_at TEXT
        );
        CREATE INDEX IF NOT EXISTS queued_session_input_pending
            ON queued_session_input(session_id, state, sequence);",
    )?;
    Ok(())
}

/// Retry accounting, so a retry has to be earned rather than assumed.
///
/// `worker_retry_budget` is keyed by objective rather than by session: retrying
/// the same objective through a fresh worker is the same spend, and counting per
/// session let an identical task be paid for again under a new id.
/// `recovery_turns` records the three kinds of turn Bridge spends on its own
/// recovery separately, because "the agent used 40 turns" and "the agent used 12
/// turns and 28 corrections" are very different bills.
fn migration_28_evidence_based_retries(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS worker_retry_budget (
            objective_key TEXT PRIMARY KEY,
            parent_session_id TEXT NOT NULL,
            attempts INTEGER NOT NULL DEFAULT 0,
            last_signal TEXT,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS recovery_turns (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            detail TEXT,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS recovery_turns_by_session
            ON recovery_turns(session_id, kind);",
    )?;
    Ok(())
}

fn migration_30_session_entry_fts(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    crate::session_recall::install_fts(transaction)
}

fn migration_1_current_schema(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS projects (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            path TEXT NOT NULL UNIQUE,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS workspaces (
            id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL REFERENCES projects(id),
            city TEXT NOT NULL,
            title TEXT NOT NULL,
            branch TEXT NOT NULL,
            path TEXT NOT NULL UNIQUE,
            status TEXT NOT NULL,
            dirty_files INTEGER NOT NULL DEFAULT 0,
            additions INTEGER NOT NULL DEFAULT 0,
            deletions INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL REFERENCES workspaces(id),
            harness TEXT NOT NULL,
            label TEXT NOT NULL,
            status TEXT NOT NULL,
            started_at TEXT,
            ended_at TEXT,
            context_percent INTEGER,
            usage_percent INTEGER,
            metric_source TEXT NOT NULL DEFAULT 'estimated'
        );
        CREATE TABLE IF NOT EXISTS events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source TEXT NOT NULL,
            kind TEXT NOT NULL,
            entity_id TEXT NOT NULL,
            body TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS agent_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL REFERENCES sessions(id),
            sequence INTEGER NOT NULL,
            protocol_version INTEGER NOT NULL DEFAULT 1,
            kind TEXT NOT NULL,
            item_id TEXT,
            role TEXT,
            status TEXT,
            title TEXT,
            text TEXT,
            data TEXT NOT NULL DEFAULT '{}',
            provider_meta TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            UNIQUE(session_id, sequence)
        );
        CREATE INDEX IF NOT EXISTS idx_agent_events_session ON agent_events(session_id, sequence);",
    )?;
    add_column_if_missing(transaction, "sessions", "provider_session_id", "TEXT")?;
    add_column_if_missing(transaction, "sessions", "active_turn_id", "TEXT")?;
    add_column_if_missing(transaction, "sessions", "model", "TEXT")?;
    add_column_if_missing(transaction, "sessions", "effort", "TEXT")?;
    add_column_if_missing(transaction, "sessions", "parent_session_id", "TEXT")?;
    add_column_if_missing(transaction, "sessions", "depth", "INTEGER")?;
    Ok(())
}

fn add_column_if_missing(
    transaction: &Transaction<'_>,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), BridgeError> {
    if !table_exists(transaction, table)? {
        return Ok(());
    }
    if !column_exists(transaction, table, column)? {
        transaction.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {definition}"
        ))?;
    }
    Ok(())
}

fn table_exists(transaction: &Transaction<'_>, table: &str) -> Result<bool, BridgeError> {
    Ok(transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        params![table],
        |row| row.get(0),
    )?)
}

fn column_exists(
    transaction: &Transaction<'_>,
    table: &str,
    column: &str,
) -> Result<bool, BridgeError> {
    let mut statement = transaction.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(columns.iter().any(|existing| existing == column))
}

fn migration_2_session_forest(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE session_entries (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL REFERENCES sessions(id),
            parent_entry_id TEXT REFERENCES session_entries(id),
            sequence INTEGER NOT NULL,
            kind TEXT NOT NULL,
            payload TEXT NOT NULL DEFAULT '{}',
            provider_event_id TEXT,
            context_visibility TEXT NOT NULL DEFAULT 'eligible',
            token_estimate INTEGER,
            created_at TEXT NOT NULL,
            UNIQUE(session_id, sequence)
        );
        CREATE INDEX idx_session_entries_parent
            ON session_entries(session_id, parent_entry_id);
        CREATE TABLE session_heads (
            session_id TEXT PRIMARY KEY REFERENCES sessions(id),
            active_entry_id TEXT REFERENCES session_entries(id),
            native_provider_session_id TEXT,
            restoration_mode TEXT NOT NULL DEFAULT 'fresh',
            latest_checkpoint_entry_id TEXT REFERENCES session_entries(id),
            updated_at TEXT NOT NULL
        );
        CREATE TABLE task_knowledge (
            id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL REFERENCES workspaces(id),
            session_id TEXT REFERENCES sessions(id),
            kind TEXT NOT NULL,
            body TEXT NOT NULL,
            source_entry_id TEXT REFERENCES session_entries(id),
            superseded_by TEXT REFERENCES task_knowledge(id),
            created_at TEXT NOT NULL
        );
        CREATE TABLE worker_leases (
            session_id TEXT PRIMARY KEY REFERENCES sessions(id),
            workspace_id TEXT NOT NULL REFERENCES workspaces(id),
            role TEXT NOT NULL,
            capability_tier TEXT NOT NULL,
            owned_paths TEXT NOT NULL DEFAULT '[]',
            write_mode TEXT NOT NULL,
            lease_status TEXT NOT NULL,
            expires_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX idx_worker_leases_workspace
            ON worker_leases(workspace_id, lease_status);
        CREATE TABLE usage_ledger (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            workspace_id TEXT NOT NULL,
            session_id TEXT,
            turn_id TEXT,
            input_tokens INTEGER,
            output_tokens INTEGER,
            cache_read_tokens INTEGER,
            cache_write_tokens INTEGER,
            context_percent INTEGER,
            capability_units INTEGER NOT NULL DEFAULT 0,
            runtime_ms INTEGER,
            source TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE INDEX idx_usage_ledger_scope
            ON usage_ledger(workspace_id, session_id, created_at);",
    )?;
    backfill_agent_events(transaction)?;
    Ok(())
}

fn migration_3_capability_tiers(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    add_column_if_missing(transaction, "sessions", "requested_tier", "TEXT")
}

fn migration_4_resume_eligibility(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    add_column_if_missing(
        transaction,
        "session_heads",
        "resume_eligibility",
        "TEXT NOT NULL DEFAULT 'fresh'",
    )
}

fn migration_5_durable_worker_pool(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    add_column_if_missing(
        transaction,
        "worker_leases",
        "task_family",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS worker_runtime (
            session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
            parent_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            lifecycle_state TEXT NOT NULL,
            task_family TEXT NOT NULL,
            compatibility_key TEXT NOT NULL,
            result_status TEXT NOT NULL DEFAULT 'pending',
            retry_count INTEGER NOT NULL DEFAULT 0,
            warm_until TEXT,
            worktree_path TEXT,
            worktree_branch TEXT,
            last_result TEXT,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_worker_runtime_parent
            ON worker_runtime(parent_session_id, result_status, lifecycle_state);
        CREATE INDEX IF NOT EXISTS idx_worker_runtime_compatibility
            ON worker_runtime(compatibility_key, lifecycle_state);
        CREATE TABLE IF NOT EXISTS delegation_receipts (
            dedupe_key TEXT PRIMARY KEY,
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            item_id TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS worker_queue (
            sequence INTEGER PRIMARY KEY AUTOINCREMENT,
            id TEXT NOT NULL UNIQUE,
            parent_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            turn_id TEXT NOT NULL,
            request TEXT NOT NULL,
            actual_model TEXT NOT NULL,
            queue_status TEXT NOT NULL DEFAULT 'queued',
            dispatched_session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_worker_queue_dispatch
            ON worker_queue(workspace_id, queue_status, sequence);",
    )?;
    Ok(())
}

fn migration_6_remove_legacy_agent_events(
    transaction: &Transaction<'_>,
) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "DROP INDEX IF EXISTS idx_agent_events_session;
         DROP TABLE IF EXISTS agent_events;",
    )?;
    Ok(())
}

/// Make git optional and support direct chats:
/// - workspaces: project_id, city, branch, path become nullable (repo-less workspaces)
/// - sessions: workspace_id becomes nullable (standalone chats); add title, kind, cwd
/// Runs with foreign keys disabled (see `open`), so the parent-table rebuilds are safe.
fn migration_7_optional_repo_and_direct_chats(
    transaction: &Transaction<'_>,
) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE workspaces_new (
            id TEXT PRIMARY KEY,
            project_id TEXT REFERENCES projects(id),
            city TEXT,
            title TEXT NOT NULL,
            branch TEXT,
            path TEXT UNIQUE,
            status TEXT NOT NULL,
            dirty_files INTEGER NOT NULL DEFAULT 0,
            additions INTEGER NOT NULL DEFAULT 0,
            deletions INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL
        );
        INSERT INTO workspaces_new (id,project_id,city,title,branch,path,status,dirty_files,additions,deletions,created_at)
            SELECT id,project_id,city,title,branch,path,status,dirty_files,additions,deletions,created_at FROM workspaces;
        DROP TABLE workspaces;
        ALTER TABLE workspaces_new RENAME TO workspaces;

        CREATE TABLE sessions_new (
            id TEXT PRIMARY KEY,
            workspace_id TEXT REFERENCES workspaces(id),
            harness TEXT NOT NULL,
            label TEXT NOT NULL,
            status TEXT NOT NULL,
            started_at TEXT,
            ended_at TEXT,
            context_percent INTEGER,
            usage_percent INTEGER,
            metric_source TEXT NOT NULL DEFAULT 'estimated',
            provider_session_id TEXT,
            active_turn_id TEXT,
            model TEXT,
            effort TEXT,
            parent_session_id TEXT,
            depth INTEGER,
            requested_tier TEXT,
            title TEXT,
            kind TEXT NOT NULL DEFAULT 'orchestrator',
            cwd TEXT
        );
        INSERT INTO sessions_new (id,workspace_id,harness,label,status,started_at,ended_at,context_percent,usage_percent,metric_source,provider_session_id,active_turn_id,model,effort,parent_session_id,depth,requested_tier)
            SELECT id,workspace_id,harness,label,status,started_at,ended_at,context_percent,usage_percent,metric_source,provider_session_id,active_turn_id,model,effort,parent_session_id,depth,requested_tier FROM sessions;
        DROP TABLE sessions;
        ALTER TABLE sessions_new RENAME TO sessions;",
    )?;
    Ok(())
}

fn migration_8_reliability_primitives(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    add_column_if_missing(
        transaction,
        "worker_queue",
        "attempt_count",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(transaction, "worker_queue", "expires_at", "TEXT")?;
    add_column_if_missing(transaction, "worker_queue", "claimed_at", "TEXT")?;
    add_column_if_missing(transaction, "worker_queue", "last_error", "TEXT")?;
    add_column_if_missing(transaction, "sessions", "trace_id", "TEXT")?;
    add_column_if_missing(transaction, "usage_ledger", "trace_id", "TEXT")?;
    transaction.execute_batch("UPDATE worker_queue SET expires_at=COALESCE(expires_at,datetime(created_at, '+24 hours'));
        CREATE INDEX IF NOT EXISTS idx_worker_queue_lifecycle ON worker_queue(queue_status,expires_at,claimed_at,sequence);
        CREATE TABLE IF NOT EXISTS durable_outbox (id TEXT PRIMARY KEY,destination TEXT NOT NULL,event_type TEXT NOT NULL,payload TEXT NOT NULL,idempotency_key TEXT NOT NULL UNIQUE,status TEXT NOT NULL DEFAULT 'pending',attempt_count INTEGER NOT NULL DEFAULT 0,next_attempt_at TEXT NOT NULL,last_error TEXT,created_at TEXT NOT NULL,delivered_at TEXT);
        CREATE INDEX IF NOT EXISTS idx_durable_outbox_delivery ON durable_outbox(status,next_attempt_at);
        CREATE TABLE IF NOT EXISTS integration_inbox (source TEXT NOT NULL,idempotency_key TEXT NOT NULL,received_at TEXT NOT NULL,PRIMARY KEY(source,idempotency_key));
        CREATE TABLE IF NOT EXISTS handoff_packets (id TEXT PRIMARY KEY,schema_version INTEGER NOT NULL,trace_id TEXT NOT NULL,source_harness TEXT NOT NULL,target_harness TEXT NOT NULL,payload TEXT NOT NULL,created_at TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS telemetry_spans (span_id TEXT PRIMARY KEY,trace_id TEXT NOT NULL,parent_span_id TEXT,name TEXT NOT NULL,attributes TEXT NOT NULL,started_at TEXT NOT NULL,ended_at TEXT);")?;
    Ok(())
}

fn migration_9_semantic_event_version(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    add_column_if_missing(
        transaction,
        "session_entries",
        "semantic_schema_version",
        "INTEGER NOT NULL DEFAULT 1",
    )
}

fn migration_10_continuation_fidelity(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    add_column_if_missing(
        transaction,
        "sessions",
        "continuation_fidelity",
        "TEXT NOT NULL DEFAULT 'native'",
    )?;
    transaction.execute_batch(
        "UPDATE sessions SET continuation_fidelity=CASE
            WHEN parent_session_id IS NULL THEN 'native'
            WHEN id IN (SELECT session_id FROM session_heads WHERE restoration_mode='checkpoint_restored') THEN 'projected_at_boundary'
            WHEN id IN (SELECT session_id FROM session_heads WHERE restoration_mode IN ('native','hot')) THEN 'native'
            ELSE 'projected_mid_turn'
         END;",
    )?;
    Ok(())
}

fn migration_11_human_blocked_queue(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    add_column_if_missing(transaction, "worker_queue", "blocked_at", "TEXT")
}

fn migration_12_adapter_process_claims(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    add_column_if_missing(transaction, "sessions", "adapter_pid", "INTEGER")?;
    add_column_if_missing(transaction, "sessions", "adapter_process_identity", "TEXT")
}

fn migration_13_learning_router(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS router_preferences (
            workspace_id TEXT PRIMARY KEY REFERENCES workspaces(id) ON DELETE CASCADE,
            mode TEXT NOT NULL,
            preferences TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS router_decisions (
            id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            parent_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            turn_id TEXT NOT NULL,
            task_family TEXT NOT NULL,
            mode TEXT NOT NULL,
            manual_override INTEGER NOT NULL,
            baseline_candidate TEXT,
            recommended_candidate TEXT,
            executed_candidate TEXT,
            decision TEXT NOT NULL,
            policy_outcome TEXT,
            route_status TEXT,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_router_decisions_workspace
            ON router_decisions(workspace_id,created_at);
        CREATE INDEX IF NOT EXISTS idx_router_decisions_turn
            ON router_decisions(parent_session_id,turn_id);
        CREATE TABLE IF NOT EXISTS router_assignments (
            decision_id TEXT PRIMARY KEY REFERENCES router_decisions(id) ON DELETE CASCADE,
            child_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            status TEXT NOT NULL,
            created_at TEXT NOT NULL,
            completed_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_router_assignments_child
            ON router_assignments(child_session_id,status,created_at);
        CREATE TABLE IF NOT EXISTS router_outcomes (
            decision_id TEXT PRIMARY KEY REFERENCES router_decisions(id) ON DELETE CASCADE,
            child_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            candidate TEXT NOT NULL,
            succeeded INTEGER NOT NULL,
            status TEXT NOT NULL,
            runtime_ms INTEGER NOT NULL,
            normalized_cost INTEGER NOT NULL,
            retry_count INTEGER NOT NULL,
            human_intervention INTEGER NOT NULL DEFAULT 0,
            recorded_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_router_outcomes_candidate
            ON router_outcomes(candidate,recorded_at);",
    )?;
    Ok(())
}

fn migration_14_completion_proof(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS completion_contracts (
            id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            schema_version INTEGER NOT NULL,
            acceptance_criteria TEXT NOT NULL,
            markdown_projection TEXT,
            markdown_committed INTEGER NOT NULL DEFAULT 0,
            status TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_completion_contracts_session
            ON completion_contracts(session_id,status,created_at);
        CREATE TABLE IF NOT EXISTS eval_plans (
            id TEXT PRIMARY KEY,
            contract_id TEXT NOT NULL REFERENCES completion_contracts(id) ON DELETE CASCADE,
            schema_version INTEGER NOT NULL,
            risk TEXT NOT NULL,
            plan TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS eval_attempts (
            id TEXT PRIMARY KEY,
            plan_id TEXT NOT NULL REFERENCES eval_plans(id) ON DELETE CASCADE,
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            repository_head TEXT NOT NULL,
            dirty_digest TEXT NOT NULL,
            repository_path TEXT NOT NULL,
            status TEXT NOT NULL,
            implementer_family TEXT,
            started_at TEXT NOT NULL,
            completed_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_eval_attempts_session
            ON eval_attempts(session_id,status,started_at);
        CREATE TABLE IF NOT EXISTS eval_check_runs (
            id TEXT PRIMARY KEY,
            attempt_id TEXT NOT NULL REFERENCES eval_attempts(id) ON DELETE CASCADE,
            check_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            required INTEGER NOT NULL,
            status TEXT NOT NULL,
            executor TEXT NOT NULL,
            command TEXT,
            verifier_family TEXT,
            detail TEXT,
            output_digest TEXT,
            artifact_refs TEXT NOT NULL DEFAULT '[]',
            started_at TEXT,
            completed_at TEXT,
            UNIQUE(attempt_id,check_id)
        );
        CREATE INDEX IF NOT EXISTS idx_eval_check_runs_attempt
            ON eval_check_runs(attempt_id,status,required);
        CREATE TABLE IF NOT EXISTS eval_findings (
            id TEXT PRIMARY KEY,
            attempt_id TEXT NOT NULL REFERENCES eval_attempts(id) ON DELETE CASCADE,
            check_id TEXT NOT NULL,
            severity TEXT NOT NULL,
            summary TEXT NOT NULL,
            affected_paths TEXT NOT NULL DEFAULT '[]',
            resolved_at TEXT,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS eval_waivers (
            id TEXT PRIMARY KEY,
            attempt_id TEXT NOT NULL REFERENCES eval_attempts(id) ON DELETE CASCADE,
            check_ids TEXT NOT NULL,
            reason TEXT NOT NULL,
            granted_by TEXT NOT NULL,
            repository_head TEXT NOT NULL,
            dirty_digest TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS proof_bundles (
            id TEXT PRIMARY KEY,
            attempt_id TEXT NOT NULL UNIQUE REFERENCES eval_attempts(id) ON DELETE CASCADE,
            schema_version INTEGER NOT NULL,
            verdict TEXT NOT NULL,
            bundle TEXT NOT NULL,
            bundle_digest TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS verifier_manifests (
            id TEXT PRIMARY KEY,
            source TEXT NOT NULL,
            schema_version INTEGER NOT NULL,
            manifest TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS worker_completion_inputs (
            child_session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
            request TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
    )?;
    Ok(())
}

fn migration_15_role_profiles_and_learning_jobs(
    transaction: &Transaction<'_>,
) -> Result<(), BridgeError> {
    add_column_if_missing(transaction, "usage_ledger", "cost_microusd", "INTEGER")?;
    add_column_if_missing(transaction, "usage_ledger", "cost_source", "TEXT")?;
    add_column_if_missing(
        transaction,
        "router_decisions",
        "task_fingerprint",
        "TEXT NOT NULL DEFAULT 'legacy'",
    )?;
    add_column_if_missing(transaction, "router_decisions", "trace_id", "TEXT")?;
    add_column_if_missing(
        transaction,
        "router_decisions",
        "repository_revision",
        "TEXT",
    )?;
    add_column_if_missing(
        transaction,
        "router_decisions",
        "profile_version",
        "INTEGER",
    )?;
    add_column_if_missing(transaction, "router_decisions", "profile_purpose", "TEXT")?;
    add_column_if_missing(
        transaction,
        "router_decisions",
        "policy_version",
        "INTEGER NOT NULL DEFAULT 1",
    )?;
    add_column_if_missing(
        transaction,
        "router_decisions",
        "catalog_snapshot",
        "TEXT NOT NULL DEFAULT '{}'",
    )?;
    add_column_if_missing(transaction, "router_decisions", "selection_reason", "TEXT")?;
    add_column_if_missing(transaction, "router_decisions", "actual_provider", "TEXT")?;
    add_column_if_missing(transaction, "router_decisions", "actual_model", "TEXT")?;
    add_column_if_missing(transaction, "router_decisions", "actual_effort", "TEXT")?;
    add_column_if_missing(
        transaction,
        "router_outcomes",
        "success_state",
        "TEXT NOT NULL DEFAULT 'unknown'",
    )?;
    add_column_if_missing(
        transaction,
        "router_outcomes",
        "acceptance_state",
        "TEXT NOT NULL DEFAULT 'unknown'",
    )?;
    add_column_if_missing(transaction, "router_outcomes", "cost_microusd", "INTEGER")?;
    add_column_if_missing(transaction, "router_outcomes", "cost_source", "TEXT")?;
    add_column_if_missing(transaction, "router_outcomes", "confidence_bps", "INTEGER")?;
    add_column_if_missing(
        transaction,
        "router_outcomes",
        "edit_count",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        transaction,
        "router_outcomes",
        "override_signal",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(transaction, "router_outcomes", "total_tokens", "INTEGER")?;
    add_column_if_missing(transaction, "router_outcomes", "latency_source", "TEXT")?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS model_profiles (
            version INTEGER NOT NULL,
            profile_id TEXT NOT NULL DEFAULT 'legacy',
            purpose TEXT NOT NULL,
            canonical_role TEXT NOT NULL,
            provider TEXT NOT NULL,
            model TEXT NOT NULL,
            effort TEXT NOT NULL,
            fallback_purpose TEXT,
            pinned INTEGER NOT NULL DEFAULT 0,
            learning_enabled INTEGER NOT NULL DEFAULT 1,
            budget_preference TEXT,
            latency_preference TEXT,
            created_at TEXT NOT NULL,
            PRIMARY KEY(version,purpose)
        );
        CREATE INDEX IF NOT EXISTS idx_model_profiles_purpose
            ON model_profiles(purpose,version);
        CREATE TABLE IF NOT EXISTS model_setup_state (
            id TEXT PRIMARY KEY,
            active_version INTEGER NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS routing_policies (
            version INTEGER PRIMARY KEY,
            status TEXT NOT NULL,
            predecessor INTEGER REFERENCES routing_policies(version),
            rollback_of INTEGER REFERENCES routing_policies(version),
            weights TEXT NOT NULL,
            thresholds TEXT NOT NULL,
            replay_report TEXT,
            created_reason TEXT NOT NULL,
            created_at TEXT NOT NULL,
            promoted_at TEXT,
            activation_boundary INTEGER
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_routing_policy_active
            ON routing_policies((1)) WHERE status IN ('active','canary');
        CREATE TABLE IF NOT EXISTS routing_evaluations (
            id TEXT PRIMARY KEY,
            learning_run_id TEXT REFERENCES learning_job_runs(id) ON DELETE SET NULL,
            decision_id TEXT REFERENCES router_decisions(id) ON DELETE CASCADE,
            evaluator_kind TEXT NOT NULL,
            evaluator_version TEXT NOT NULL,
            score_bps INTEGER,
            confidence_bps INTEGER,
            evidence_entry_ids TEXT NOT NULL DEFAULT '[]',
            bounded_metrics TEXT NOT NULL DEFAULT '{}',
            status TEXT NOT NULL DEFAULT 'completed',
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS learning_jobs (
            id TEXT PRIMARY KEY,
            enabled INTEGER NOT NULL DEFAULT 0,
            cadence_minutes INTEGER NOT NULL DEFAULT 1440,
            next_run_at TEXT,
            last_evidence_boundary INTEGER NOT NULL DEFAULT 0,
            run_budget_microusd INTEGER NOT NULL DEFAULT 100000,
            run_budget_tokens INTEGER NOT NULL DEFAULT 50000,
            mode TEXT NOT NULL DEFAULT 'manual',
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS learning_triggers (
            id TEXT PRIMARY KEY,
            job_id TEXT NOT NULL REFERENCES learning_jobs(id) ON DELETE CASCADE,
            kind TEXT NOT NULL,
            registration_id TEXT NOT NULL,
            credential_ref TEXT,
            auth_digest TEXT,
            enabled INTEGER NOT NULL DEFAULT 1,
            expires_at TEXT,
            experimental INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE(kind,registration_id)
        );
        CREATE TABLE IF NOT EXISTS learning_job_runs (
            id TEXT PRIMARY KEY,
            job_id TEXT NOT NULL REFERENCES learning_jobs(id) ON DELETE CASCADE,
            trigger_kind TEXT NOT NULL,
            idempotency_key TEXT NOT NULL UNIQUE,
            evidence_boundary INTEGER NOT NULL,
            base_policy_version INTEGER NOT NULL,
            status TEXT NOT NULL,
            lease_owner TEXT,
            lease_expires_at TEXT,
            snapshot_frozen_at TEXT NOT NULL,
            report TEXT,
            candidate_policy_version INTEGER REFERENCES routing_policies(version),
            cancellation_requested INTEGER NOT NULL DEFAULT 0,
            evaluated_spend_microusd INTEGER NOT NULL DEFAULT 0,
            evaluated_tokens INTEGER NOT NULL DEFAULT 0,
            replay_passed INTEGER,
            promotion_status TEXT NOT NULL DEFAULT 'not_requested',
            created_at TEXT NOT NULL,
            completed_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_learning_job_runs_status
            ON learning_job_runs(job_id,status,created_at);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_learning_job_active_lease
            ON learning_job_runs(job_id) WHERE status IN ('queued','running');
        CREATE TABLE IF NOT EXISTS learning_trigger_events (
            id TEXT PRIMARY KEY,
            run_id TEXT REFERENCES learning_job_runs(id) ON DELETE SET NULL,
            trigger_kind TEXT NOT NULL,
            registration_id TEXT,
            result TEXT NOT NULL,
            reason TEXT,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS routing_policy_promotions (
            id TEXT PRIMARY KEY,
            from_version INTEGER NOT NULL REFERENCES routing_policies(version),
            to_version INTEGER NOT NULL REFERENCES routing_policies(version),
            learning_run_id TEXT REFERENCES learning_job_runs(id) ON DELETE SET NULL,
            action TEXT NOT NULL,
            actor TEXT NOT NULL,
            explanation TEXT NOT NULL,
            replay_report TEXT,
            created_at TEXT NOT NULL
        );
        INSERT OR IGNORE INTO routing_policies(version,status,weights,thresholds,created_reason,created_at)
            VALUES(1,'active','{}','{}','initial deterministic routing policy',CURRENT_TIMESTAMP);
        INSERT OR IGNORE INTO learning_jobs(id,enabled,cadence_minutes,run_budget_microusd,run_budget_tokens,mode,updated_at)
            VALUES('default',0,1440,100000,50000,'manual',CURRENT_TIMESTAMP);
        INSERT OR IGNORE INTO learning_triggers(id,job_id,kind,registration_id,enabled,experimental,created_at,updated_at)
            VALUES('builtin-manual','default','manual','built-in',1,0,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);
        INSERT OR IGNORE INTO learning_triggers(id,job_id,kind,registration_id,enabled,experimental,created_at,updated_at)
            VALUES('builtin-in-app','default','in_app','built-in',1,0,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);",
    )?;
    add_column_if_missing(
        transaction,
        "model_profiles",
        "profile_id",
        "TEXT NOT NULL DEFAULT 'legacy'",
    )?;
    add_column_if_missing(
        transaction,
        "learning_jobs",
        "last_evidence_boundary",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    transaction.execute(
        "CREATE INDEX IF NOT EXISTS idx_model_profiles_id ON model_profiles(profile_id,version)",
        [],
    )?;
    Ok(())
}

fn migration_16_complete_role_profile_schema(
    transaction: &Transaction<'_>,
) -> Result<(), BridgeError> {
    // These columns were added to migration 15 after some databases had
    // already recorded version 15. A new migration is required to repair
    // those databases because completed migrations are never replayed.
    add_column_if_missing(
        transaction,
        "model_profiles",
        "profile_id",
        "TEXT NOT NULL DEFAULT 'legacy'",
    )?;
    add_column_if_missing(
        transaction,
        "learning_jobs",
        "last_evidence_boundary",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    transaction.execute(
        "CREATE INDEX IF NOT EXISTS idx_model_profiles_id ON model_profiles(profile_id,version)",
        [],
    )?;
    Ok(())
}

#[derive(Debug)]
struct LegacyAgentEvent {
    id: i64,
    session_id: String,
    sequence: i64,
    protocol_version: i64,
    kind: String,
    item_id: Option<String>,
    role: Option<String>,
    status: Option<String>,
    title: Option<String>,
    text: Option<String>,
    data: String,
    provider_meta: String,
    created_at: String,
}

fn backfill_agent_events(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    let legacy_events = {
        let mut statement = transaction.prepare(
            "SELECT id,session_id,sequence,protocol_version,kind,item_id,role,status,title,text,data,provider_meta,created_at
             FROM agent_events ORDER BY session_id,sequence,id",
        )?;
        let events = statement
            .query_map([], |row| {
                Ok(LegacyAgentEvent {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    sequence: row.get(2)?,
                    protocol_version: row.get(3)?,
                    kind: row.get(4)?,
                    item_id: row.get(5)?,
                    role: row.get(6)?,
                    status: row.get(7)?,
                    title: row.get(8)?,
                    text: row.get(9)?,
                    data: row.get(10)?,
                    provider_meta: row.get(11)?,
                    created_at: row.get(12)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        events
    };
    let mut previous_by_session: HashMap<String, String> = HashMap::new();
    for event in legacy_events {
        let entry_id = format!("agent-event-{}", event.id);
        let parent_entry_id = previous_by_session.get(&event.session_id).cloned();
        let data = serde_json::from_str(&event.data).unwrap_or(serde_json::Value::Null);
        let provider_meta =
            serde_json::from_str(&event.provider_meta).unwrap_or(serde_json::Value::Null);
        let payload = serde_json::json!({
            "protocolVersion": event.protocol_version,
            "itemId": event.item_id,
            "role": event.role,
            "status": event.status,
            "title": event.title,
            "text": event.text,
            "data": data,
            "providerMeta": provider_meta,
        });
        transaction.execute(
            "INSERT INTO session_entries(id,session_id,parent_entry_id,sequence,kind,payload,provider_event_id,created_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                entry_id,
                event.session_id,
                parent_entry_id,
                event.sequence,
                event.kind,
                payload.to_string(),
                event.item_id,
                event.created_at
            ],
        )?;
        previous_by_session.insert(event.session_id, entry_id);
    }
    let now = Utc::now().to_rfc3339();
    transaction.execute(
        "INSERT INTO session_heads(session_id,active_entry_id,native_provider_session_id,restoration_mode,updated_at)
         SELECT sessions.id,
                (SELECT id FROM session_entries WHERE session_id=sessions.id ORDER BY sequence DESC LIMIT 1),
                sessions.provider_session_id,
                'fresh',
                ?1
         FROM sessions",
        params![now],
    )?;
    Ok(())
}

pub fn state(db: &Connection) -> Result<BridgeState, BridgeError> {
    let projects = query(
        db,
        "SELECT id,name,path,created_at FROM projects ORDER BY created_at",
        |r| {
            Ok(Project {
                id: r.get(0)?,
                name: r.get(1)?,
                path: r.get(2)?,
                created_at: r.get(3)?,
            })
        },
    )?;
    let workspaces = query(db, "SELECT id,project_id,city,title,branch,path,status,dirty_files,additions,deletions,created_at FROM workspaces ORDER BY created_at", |r| Ok(Workspace { id:r.get(0)?, project_id:r.get(1)?, city:r.get(2)?, title:r.get(3)?, branch:r.get(4)?, path:r.get(5)?, status:status(&r.get::<_,String>(6)?), dirty_files:r.get(7)?, additions:r.get(8)?, deletions:r.get(9)?, created_at:r.get(10)? }))?;
    let sessions = query(db, "SELECT s.id,s.workspace_id,s.harness,s.label,s.status,s.started_at,s.ended_at,s.context_percent,s.usage_percent,s.metric_source,s.provider_session_id,s.active_turn_id,s.model,s.requested_tier,s.effort,s.parent_session_id,s.depth,COALESCE(h.restoration_mode,'fresh'),s.continuation_fidelity,s.title,s.kind,s.cwd FROM sessions s LEFT JOIN session_heads h ON h.session_id=s.id ORDER BY s.rowid", |r| Ok(Session { id:r.get(0)?, workspace_id:r.get(1)?, harness:harness(&r.get::<_,String>(2)?), label:r.get(3)?, status:status(&r.get::<_,String>(4)?), started_at:r.get(5)?, ended_at:r.get(6)?, context_percent:r.get(7)?, usage_percent:r.get(8)?, metric_source:r.get(9)?, provider_session_id:r.get(10)?, active_turn_id:r.get(11)?, model:r.get(12)?, requested_tier:capability_tier(r.get::<_,Option<String>>(13)?), effort:r.get(14)?, parent_session_id:r.get(15)?, depth:r.get(16)?, restoration_mode:restoration_mode(&r.get::<_,String>(17)?), continuation_fidelity:continuation_fidelity(&r.get::<_,String>(18)?), title:r.get(19)?, kind:r.get(20)?, cwd:r.get(21)? }))?;
    let events = query(
        db,
        "SELECT id,source,kind,entity_id,body,created_at FROM events ORDER BY id DESC LIMIT 200",
        |r| {
            Ok(BridgeEvent {
                id: r.get(0)?,
                source: r.get(1)?,
                kind: r.get(2)?,
                entity_id: r.get(3)?,
                body: r.get(4)?,
                created_at: r.get(5)?,
            })
        },
    )?;
    Ok(BridgeState {
        projects,
        workspaces,
        sessions,
        events,
    })
}

fn query<T, F>(db: &Connection, sql: &str, mut map: F) -> Result<Vec<T>, BridgeError>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    let mut stmt = db.prepare(sql)?;
    let rows = stmt.query_map([], |row| map(row))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}
pub fn event(
    db: &Connection,
    source: &str,
    kind: &str,
    entity: &str,
    body: &str,
) -> Result<(), BridgeError> {
    db.execute(
        "INSERT INTO events(source,kind,entity_id,body,created_at) VALUES(?1,?2,?3,?4,?5)",
        params![source, kind, entity, body, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}
pub fn status(value: &str) -> SessionStatus {
    match value {
        "starting" => SessionStatus::Starting,
        "working" => SessionStatus::Working,
        "waiting" => SessionStatus::Waiting,
        "warm" => SessionStatus::Warm,
        "checkpointing" => SessionStatus::Checkpointing,
        "ready" => SessionStatus::Ready,
        "failed" => SessionStatus::Failed,
        "stopped" => SessionStatus::Stopped,
        "resuming" => SessionStatus::Resuming,
        "restored" => SessionStatus::Restored,
        "completed" => SessionStatus::Completed,
        "cancelled" => SessionStatus::Cancelled,
        _ => SessionStatus::Idle,
    }
}
/// Interpret the `harness` column. Never guesses: an id this build cannot
/// parse is preserved as [`Harness::Unknown`] rather than being read as some
/// other harness. See [`Harness::from_stored`].
pub fn harness(value: &str) -> Harness {
    Harness::from_stored(value)
}
fn capability_tier(value: Option<String>) -> Option<CapabilityTier> {
    match value.as_deref() {
        Some("fast") => Some(CapabilityTier::Fast),
        Some("standard") => Some(CapabilityTier::Standard),
        Some("strong") => Some(CapabilityTier::Strong),
        _ => None,
    }
}
fn restoration_mode(value: &str) -> RestorationMode {
    match value {
        "hot" => RestorationMode::Hot,
        "native" => RestorationMode::Native,
        "checkpoint_restored" => RestorationMode::CheckpointRestored,
        _ => RestorationMode::Fresh,
    }
}
fn resume_eligibility(value: &str) -> ResumeEligibility {
    match value {
        "native" => ResumeEligibility::Native,
        "checkpoint_restored" => ResumeEligibility::CheckpointRestored,
        _ => ResumeEligibility::Fresh,
    }
}

fn continuation_fidelity(value: &str) -> ContinuationFidelity {
    match value {
        "projected_at_boundary" => ContinuationFidelity::ProjectedAtBoundary,
        "projected_mid_turn" => ContinuationFidelity::ProjectedMidTurn,
        _ => ContinuationFidelity::Native,
    }
}
/// The value written to the `harness` column, which is also the wire id and
/// the adapter-registry key. See [`Harness::id`].
pub fn harness_name(value: &Harness) -> std::borrow::Cow<'static, str> {
    value.id()
}

#[allow(clippy::too_many_arguments)]
pub fn append_session_entry(
    db: &Connection,
    session_id: &str,
    parent_entry_id: Option<&str>,
    kind: &str,
    payload: &serde_json::Value,
    provider_event_id: Option<&str>,
    context_visibility: &str,
    token_estimate: Option<i64>,
) -> Result<SessionEntry, BridgeError> {
    let transaction = db.unchecked_transaction()?;
    let entry = append_session_entry_tx(
        &transaction,
        session_id,
        parent_entry_id,
        kind,
        payload,
        provider_event_id,
        context_visibility,
        token_estimate,
    )?;
    transaction.commit()?;
    Ok(entry)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn append_session_entry_tx(
    transaction: &Transaction<'_>,
    session_id: &str,
    parent_entry_id: Option<&str>,
    kind: &str,
    payload: &serde_json::Value,
    provider_event_id: Option<&str>,
    context_visibility: &str,
    token_estimate: Option<i64>,
) -> Result<SessionEntry, BridgeError> {
    let sequence = transaction.query_row(
        "SELECT COALESCE(MAX(sequence),0)+1 FROM session_entries WHERE session_id=?1",
        params![session_id],
        |row| row.get(0),
    )?;
    let mut stored_payload = payload.clone();
    let object = stored_payload.as_object_mut().ok_or_else(|| {
        BridgeError::Invalid("session entry payload must be a JSON object".into())
    })?;
    object.insert(
        "_bridgeRepoState".into(),
        repository_state_for_session(transaction, session_id)?,
    );
    let entry = SessionEntry {
        id: Uuid::new_v4().to_string(),
        session_id: session_id.to_owned(),
        parent_entry_id: parent_entry_id.map(str::to_owned),
        sequence,
        semantic_schema_version: SEMANTIC_EVENT_SCHEMA_VERSION,
        kind: kind.to_owned(),
        payload: stored_payload,
        provider_event_id: provider_event_id.map(str::to_owned),
        context_visibility: context_visibility.to_owned(),
        token_estimate,
        created_at: Utc::now().to_rfc3339(),
    };
    transaction.execute(
        "INSERT INTO session_entries(id,session_id,parent_entry_id,sequence,semantic_schema_version,kind,payload,provider_event_id,context_visibility,token_estimate,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            entry.id,
            entry.session_id,
            entry.parent_entry_id,
            entry.sequence,
            entry.semantic_schema_version,
            entry.kind,
            entry.payload.to_string(),
            entry.provider_event_id,
            entry.context_visibility,
            entry.token_estimate,
            entry.created_at,
        ],
    )?;
    transaction.execute(
        "INSERT INTO session_heads(session_id,active_entry_id,native_provider_session_id,restoration_mode,updated_at)
         VALUES(?1,?2,(SELECT provider_session_id FROM sessions WHERE id=?1),'fresh',?3)
         ON CONFLICT(session_id) DO UPDATE SET active_entry_id=excluded.active_entry_id,updated_at=excluded.updated_at",
        params![entry.session_id, entry.id, entry.created_at],
    )?;
    Ok(entry)
}

pub fn repository_state_for_session(
    db: &Connection,
    session_id: &str,
) -> Result<serde_json::Value, BridgeError> {
    let Some(path) = repository_path_for_session(db, session_id)? else {
        return Ok(serde_json::json!({"status":"unavailable"}));
    };
    Ok(repository_state_for_path(&path))
}

pub fn repository_path_for_session(
    db: &Connection,
    session_id: &str,
) -> Result<Option<PathBuf>, BridgeError> {
    let path: Option<String> = db.query_row(
        "SELECT COALESCE(s.cwd,w.path) FROM sessions s LEFT JOIN workspaces w ON w.id=s.workspace_id WHERE s.id=?1",
        params![session_id],
        |row| row.get(0),
    ).optional()?.flatten();
    Ok(path.map(PathBuf::from))
}

pub fn repository_state_for_path(path: &Path) -> serde_json::Value {
    let head = crate::git::git_command(path)
        .args(["rev-parse", "HEAD"])
        .output();
    let status = crate::git::git_command(path)
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .output();
    let (Ok(head), Ok(status)) = (head, status) else {
        return serde_json::json!({"status":"unavailable"});
    };
    if !head.status.success() || !status.status.success() {
        return serde_json::json!({"status":"unavailable"});
    }
    let head = String::from_utf8_lossy(&head.stdout).trim().to_owned();
    serde_json::json!({
        "status": if status.stdout.is_empty() { "clean" } else { "dirty" },
        "head": head,
        "dirtyHash": stable_dirty_hash(&status.stdout),
    })
}

fn stable_dirty_hash(bytes: &[u8]) -> String {
    // FNV-1a is sufficient here: this is a deterministic change detector, not a
    // security boundary. Keeping the algorithm local makes stamps comparable
    // across controller restarts and Rust versions.
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

/// Durable events for a session with a sequence strictly greater than the
/// cursor, in sequence order — the replay half of the notify-then-replay
/// contract. Replayed events carry their durable forest kind (e.g.
/// `assistant.message`) and payload exactly as persisted; transient frames
/// (sequence 0 on the live channel) were never stored and are never replayed.
pub fn session_events_after(
    db: &Connection,
    session_id: &str,
    after_sequence: i64,
    limit: u32,
) -> Result<Vec<AgentEvent>, BridgeError> {
    let entries = query_with_params(
        db,
        "SELECT id,session_id,parent_entry_id,sequence,semantic_schema_version,kind,payload,provider_event_id,context_visibility,token_estimate,created_at
         FROM session_entries WHERE session_id=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3",
        params![session_id, after_sequence, limit],
        |row| {
            Ok(SessionEntry {
                id: row.get(0)?,
                session_id: row.get(1)?,
                parent_entry_id: row.get(2)?,
                sequence: row.get(3)?,
                semantic_schema_version: row.get(4)?,
                kind: row.get(5)?,
                payload: parse_json_column(row, 6),
                provider_event_id: row.get(7)?,
                context_visibility: row.get(8)?,
                token_estimate: row.get(9)?,
                created_at: row.get(10)?,
            })
        },
    )?;
    let forest = crate::session_forest::SessionForest::new(db);
    entries
        .into_iter()
        .map(|entry| {
            forest.validate_stored_entry(&entry).map_err(|error| {
                BridgeError::Invalid(format!(
                    "cannot replay session entry {} at sequence {}: {error}",
                    entry.id, entry.sequence
                ))
            })?;
            let payload = &entry.payload;
            let field = |name: &str| {
                payload
                    .get(name)
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            };
            let typed_forest_payload = payload
                .get(crate::session_forest::TYPED_SCHEMA_MARKER)
                .and_then(serde_json::Value::as_u64)
                == Some(crate::session_forest::TYPED_SCHEMA_VERSION);
            let legacy_agent_envelope = !typed_forest_payload
                && payload
                    .get("protocolVersion")
                    .and_then(serde_json::Value::as_i64)
                    .is_some();
            Ok(AgentEvent {
                id: entry.sequence,
                session_id: entry.session_id,
                sequence: entry.sequence,
                protocol_version: if legacy_agent_envelope {
                    payload
                        .get("protocolVersion")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(1)
                } else {
                    1
                },
                kind: entry.kind,
                item_id: field("itemId"),
                role: field("role"),
                status: field("status"),
                title: field("title"),
                text: field("text"),
                data: if legacy_agent_envelope {
                    payload
                        .get("data")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({}))
                } else {
                    payload.clone()
                },
                provider_meta: payload
                    .get("providerMeta")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({})),
                created_at: entry.created_at,
            })
        })
        .collect()
}

pub fn session_entries(
    db: &Connection,
    session_id: &str,
) -> Result<Vec<SessionEntry>, BridgeError> {
    query_with_params(
        db,
        "SELECT id,session_id,parent_entry_id,sequence,semantic_schema_version,kind,payload,provider_event_id,context_visibility,token_estimate,created_at
         FROM session_entries WHERE session_id=?1 ORDER BY sequence",
        params![session_id],
        |row| {
            Ok(SessionEntry {
                id: row.get(0)?,
                session_id: row.get(1)?,
                parent_entry_id: row.get(2)?,
                sequence: row.get(3)?,
                semantic_schema_version: row.get(4)?,
                kind: row.get(5)?,
                payload: parse_json_column(row, 6),
                provider_event_id: row.get(7)?,
                context_visibility: row.get(8)?,
                token_estimate: row.get(9)?,
                created_at: row.get(10)?,
            })
        },
    )
}

pub fn session_head(db: &Connection, session_id: &str) -> Result<Option<SessionHead>, BridgeError> {
    db.query_row(
        "SELECT session_id,active_entry_id,native_provider_session_id,restoration_mode,resume_eligibility,latest_checkpoint_entry_id,updated_at
         FROM session_heads WHERE session_id=?1",
        params![session_id],
        |row| {
            Ok(SessionHead {
                session_id: row.get(0)?,
                active_entry_id: row.get(1)?,
                native_provider_session_id: row.get(2)?,
                restoration_mode: restoration_mode(&row.get::<_, String>(3)?),
                resume_eligibility: resume_eligibility(&row.get::<_, String>(4)?),
                latest_checkpoint_entry_id: row.get(5)?,
                updated_at: row.get(6)?,
            })
        },
    )
    .optional()
    .map_err(BridgeError::from)
}

pub fn insert_task_knowledge(
    db: &Connection,
    knowledge: &TaskKnowledge,
) -> Result<(), BridgeError> {
    db.execute(
        "INSERT INTO task_knowledge(id,workspace_id,session_id,kind,body,source_entry_id,superseded_by,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            knowledge.id,
            knowledge.workspace_id,
            knowledge.session_id,
            knowledge.kind,
            knowledge.body,
            knowledge.source_entry_id,
            knowledge.superseded_by,
            knowledge.created_at,
        ],
    )?;
    Ok(())
}

pub fn task_knowledge(
    db: &Connection,
    workspace_id: &str,
) -> Result<Vec<TaskKnowledge>, BridgeError> {
    query_with_params(
        db,
        "SELECT id,workspace_id,session_id,kind,body,source_entry_id,superseded_by,created_at
         FROM task_knowledge WHERE workspace_id=?1 ORDER BY created_at,id",
        params![workspace_id],
        |row| {
            Ok(TaskKnowledge {
                id: row.get(0)?,
                workspace_id: row.get(1)?,
                session_id: row.get(2)?,
                kind: row.get(3)?,
                body: row.get(4)?,
                source_entry_id: row.get(5)?,
                superseded_by: row.get(6)?,
                created_at: row.get(7)?,
            })
        },
    )
}

pub fn upsert_worker_lease(db: &Connection, lease: &WorkerLease) -> Result<(), BridgeError> {
    db.execute(
        "INSERT INTO worker_leases(session_id,workspace_id,role,capability_tier,task_family,owned_paths,write_mode,lease_status,expires_at,created_at,updated_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
         ON CONFLICT(session_id) DO UPDATE SET workspace_id=excluded.workspace_id,role=excluded.role,capability_tier=excluded.capability_tier,task_family=excluded.task_family,owned_paths=excluded.owned_paths,write_mode=excluded.write_mode,lease_status=excluded.lease_status,expires_at=excluded.expires_at,updated_at=excluded.updated_at",
        params![
            lease.session_id,
            lease.workspace_id,
            lease.role,
            lease.capability_tier,
            lease.task_family,
            lease.owned_paths.to_string(),
            lease.write_mode,
            lease.lease_status,
            lease.expires_at,
            lease.created_at,
            lease.updated_at,
        ],
    )?;
    Ok(())
}

pub fn worker_leases(db: &Connection, workspace_id: &str) -> Result<Vec<WorkerLease>, BridgeError> {
    query_with_params(
        db,
        "SELECT session_id,workspace_id,role,capability_tier,task_family,owned_paths,write_mode,lease_status,expires_at,created_at,updated_at
         FROM worker_leases WHERE workspace_id=?1 ORDER BY created_at,session_id",
        params![workspace_id],
        |row| {
            Ok(WorkerLease {
                session_id: row.get(0)?,
                workspace_id: row.get(1)?,
                role: row.get(2)?,
                capability_tier: row.get(3)?,
                task_family: row.get(4)?,
                owned_paths: parse_json_column(row, 5),
                write_mode: row.get(6)?,
                lease_status: row.get(7)?,
                expires_at: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
            })
        },
    )
}

pub fn claim_delegation_receipt(
    db: &Connection,
    session_id: &str,
    item_id: &str,
) -> Result<bool, BridgeError> {
    let dedupe_key = format!("{session_id}::{item_id}");
    Ok(db.execute(
        "INSERT OR IGNORE INTO delegation_receipts(dedupe_key,session_id,item_id,created_at) VALUES(?1,?2,?3,?4)",
        params![dedupe_key, session_id, item_id, Utc::now().to_rfc3339()],
    )? == 1)
}

pub fn upsert_worker_runtime(
    db: &Connection,
    runtime: &WorkerRuntimeRecord,
) -> Result<(), BridgeError> {
    db.execute(
        "INSERT INTO worker_runtime(session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,result_status,retry_count,warm_until,worktree_path,worktree_branch,last_result,last_activity_at,updated_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)
         ON CONFLICT(session_id) DO UPDATE SET parent_session_id=excluded.parent_session_id,lifecycle_state=excluded.lifecycle_state,task_family=excluded.task_family,compatibility_key=excluded.compatibility_key,result_status=excluded.result_status,retry_count=excluded.retry_count,warm_until=excluded.warm_until,worktree_path=excluded.worktree_path,worktree_branch=excluded.worktree_branch,last_result=excluded.last_result,last_activity_at=excluded.last_activity_at,updated_at=excluded.updated_at",
        params![runtime.session_id,runtime.parent_session_id,runtime.lifecycle_state,runtime.task_family,runtime.compatibility_key,runtime.result_status,runtime.retry_count,runtime.warm_until,runtime.worktree_path,runtime.worktree_branch,runtime.last_result.as_ref().map(serde_json::Value::to_string),runtime.last_activity_at,runtime.updated_at],
    )?;
    Ok(())
}

pub fn worker_runtime(
    db: &Connection,
    session_id: &str,
) -> Result<Option<WorkerRuntimeRecord>, BridgeError> {
    db.query_row(
        "SELECT session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,result_status,retry_count,warm_until,worktree_path,worktree_branch,last_result,last_activity_at,updated_at FROM worker_runtime WHERE session_id=?1",
        params![session_id],
        |row| Ok(WorkerRuntimeRecord { session_id:row.get(0)?, parent_session_id:row.get(1)?, lifecycle_state:row.get(2)?, task_family:row.get(3)?, compatibility_key:row.get(4)?, result_status:row.get(5)?, retry_count:row.get(6)?, warm_until:row.get(7)?, worktree_path:row.get(8)?, worktree_branch:row.get(9)?, last_result:row.get::<_,Option<String>>(10)?.and_then(|value| serde_json::from_str(&value).ok()), last_activity_at:row.get(11)?, updated_at:row.get(12)? }),
    ).optional().map_err(BridgeError::from)
}

pub fn worker_runtimes(
    db: &Connection,
    workspace_id: &str,
) -> Result<Vec<WorkerRuntimeRecord>, BridgeError> {
    query_with_params(
        db,
        "SELECT r.session_id,r.parent_session_id,r.lifecycle_state,r.task_family,r.compatibility_key,r.result_status,r.retry_count,r.warm_until,r.worktree_path,r.worktree_branch,r.last_result,r.last_activity_at,r.updated_at
         FROM worker_runtime r JOIN sessions s ON s.id=r.session_id
         WHERE s.workspace_id=?1 ORDER BY s.rowid",
        params![workspace_id],
        |row| {
            Ok(WorkerRuntimeRecord {
                session_id: row.get(0)?,
                parent_session_id: row.get(1)?,
                lifecycle_state: row.get(2)?,
                task_family: row.get(3)?,
                compatibility_key: row.get(4)?,
                result_status: row.get(5)?,
                retry_count: row.get(6)?,
                warm_until: row.get(7)?,
                worktree_path: row.get(8)?,
                worktree_branch: row.get(9)?,
                last_result: row
                    .get::<_, Option<String>>(10)?
                    .and_then(|value| serde_json::from_str(&value).ok()),
                last_activity_at: row.get(11)?,
                updated_at: row.get(12)?,
            })
        },
    )
}

pub fn outstanding_children(db: &Connection, parent_session_id: &str) -> Result<i64, BridgeError> {
    Ok(db.query_row(
        "SELECT COUNT(*) FROM worker_runtime WHERE parent_session_id=?1 AND result_status!='reported'",
        params![parent_session_id],
        |row| row.get(0),
    )?)
}

pub fn enqueue_worker_request(
    db: &Connection,
    request: &QueuedWorkerRequest,
) -> Result<(), BridgeError> {
    db.execute(
        "INSERT INTO worker_queue(id,parent_session_id,workspace_id,turn_id,request,actual_model,queue_status,dispatched_session_id,attempt_count,expires_at,blocked_at,claimed_at,last_error,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
        params![request.id,request.parent_session_id,request.workspace_id,request.turn_id,request.request.to_string(),request.actual_model,request.queue_status,request.dispatched_session_id,request.attempt_count,request.expires_at,request.blocked_at,request.claimed_at,request.last_error,request.created_at,request.updated_at],
    )?;
    Ok(())
}

pub fn queued_worker_requests(
    db: &Connection,
    workspace_id: &str,
) -> Result<Vec<QueuedWorkerRequest>, BridgeError> {
    query_with_params(
        db,
        "SELECT id,parent_session_id,workspace_id,turn_id,request,actual_model,queue_status,sequence,dispatched_session_id,attempt_count,expires_at,blocked_at,claimed_at,last_error,created_at,updated_at FROM worker_queue WHERE workspace_id=?1 AND queue_status='queued' ORDER BY sequence",
        params![workspace_id],
        |row| Ok(QueuedWorkerRequest { id:row.get(0)?, parent_session_id:row.get(1)?, workspace_id:row.get(2)?, turn_id:row.get(3)?, request:parse_json_column(row,4), actual_model:row.get(5)?, queue_status:row.get(6)?, sequence:row.get(7)?, dispatched_session_id:row.get(8)?, attempt_count:row.get(9)?, expires_at:row.get(10)?, blocked_at:row.get(11)?, claimed_at:row.get(12)?, last_error:row.get(13)?, created_at:row.get(14)?, updated_at:row.get(15)? }),
    )
}

pub fn worker_queue_requests(
    db: &Connection,
    workspace_id: &str,
) -> Result<Vec<QueuedWorkerRequest>, BridgeError> {
    query_with_params(
        db,
        "SELECT id,parent_session_id,workspace_id,turn_id,request,actual_model,queue_status,sequence,dispatched_session_id,attempt_count,expires_at,blocked_at,claimed_at,last_error,created_at,updated_at
         FROM worker_queue WHERE workspace_id=?1 ORDER BY sequence",
        params![workspace_id],
        |row| {
            Ok(QueuedWorkerRequest {
                id: row.get(0)?,
                parent_session_id: row.get(1)?,
                workspace_id: row.get(2)?,
                turn_id: row.get(3)?,
                request: parse_json_column(row, 4),
                actual_model: row.get(5)?,
                queue_status: row.get(6)?,
                sequence: row.get(7)?,
                dispatched_session_id: row.get(8)?,
                attempt_count: row.get(9)?, expires_at: row.get(10)?, blocked_at: row.get(11)?, claimed_at: row.get(12)?, last_error: row.get(13)?, created_at: row.get(14)?, updated_at: row.get(15)?,
            })
        },
    )
}

pub fn fair_queued_workspaces(db: &Connection) -> Result<Vec<String>, BridgeError> {
    query_with_params(db, "SELECT workspace_id FROM worker_queue WHERE queue_status='queued' GROUP BY workspace_id ORDER BY MIN(sequence),workspace_id", [], |row| row.get(0))
}

pub fn enqueue_outbox(
    transaction: &Transaction<'_>,
    message: &OutboxMessage,
) -> Result<(), BridgeError> {
    transaction.execute("INSERT INTO durable_outbox(id,destination,event_type,payload,idempotency_key,status,attempt_count,next_attempt_at,last_error,created_at,delivered_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) ON CONFLICT(idempotency_key) DO NOTHING", params![message.id,message.destination,message.event_type,message.payload.to_string(),message.idempotency_key,message.status,message.attempt_count,message.next_attempt_at,message.last_error,message.created_at,message.delivered_at])?;
    Ok(())
}

pub fn workspace_reason_events(
    db: &Connection,
    workspace_id: &str,
) -> Result<Vec<BridgeEvent>, BridgeError> {
    query_with_params(
        db,
        "SELECT id,source,kind,entity_id,body,created_at FROM events
         WHERE entity_id=?1
            OR entity_id IN (SELECT id FROM sessions WHERE workspace_id=?1)
            OR entity_id IN (SELECT id FROM worker_queue WHERE workspace_id=?1)
         ORDER BY id DESC",
        params![workspace_id],
        |row| {
            Ok(BridgeEvent {
                id: row.get(0)?,
                source: row.get(1)?,
                kind: row.get(2)?,
                entity_id: row.get(3)?,
                body: row.get(4)?,
                created_at: row.get(5)?,
            })
        },
    )
}

pub fn update_worker_queue(
    db: &Connection,
    id: &str,
    queue_status: &str,
    dispatched_session_id: Option<&str>,
) -> Result<bool, BridgeError> {
    Ok(db.execute(
        "UPDATE worker_queue SET queue_status=?2,dispatched_session_id=COALESCE(?3,dispatched_session_id),updated_at=?4 WHERE id=?1",
        params![id, queue_status, dispatched_session_id, Utc::now().to_rfc3339()],
    )? == 1)
}

pub fn append_usage_ledger(db: &Connection, usage: &UsageLedgerRow) -> Result<i64, BridgeError> {
    db.execute(
        "INSERT INTO usage_ledger(workspace_id,session_id,turn_id,input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,uncached_input_tokens,context_percent,capability_units,runtime_ms,cost_microusd,cost_source,stable_prefix_id,stable_prefix_hash,prompt_schema_version,prefix_token_estimate,harness,model,role,task_family,restoration_mode,cross_harness_reuse,source,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25)",
        params![
            usage.workspace_id,
            usage.session_id,
            usage.turn_id,
            usage.input_tokens,
            usage.output_tokens,
            usage.cache_read_tokens,
            usage.cache_write_tokens,
            usage.uncached_input_tokens,
            usage.context_percent,
            usage.capability_units,
            usage.runtime_ms,
            usage.cost_microusd,
            usage.cost_source,
            usage.stable_prefix_id,
            usage.stable_prefix_hash,
            usage.prompt_schema_version,
            usage.prefix_token_estimate,
            usage.harness,
            usage.model,
            usage.role,
            usage.task_family,
            usage.restoration_mode,
            usage.cross_harness_reuse,
            usage.source,
            usage.created_at,
        ],
    )?;
    Ok(db.last_insert_rowid())
}

pub fn usage_ledger(
    db: &Connection,
    workspace_id: &str,
    session_id: Option<&str>,
) -> Result<Vec<UsageLedgerRow>, BridgeError> {
    let sql = "SELECT id,workspace_id,session_id,turn_id,input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,uncached_input_tokens,context_percent,capability_units,runtime_ms,cost_microusd,cost_source,stable_prefix_id,stable_prefix_hash,prompt_schema_version,prefix_token_estimate,harness,model,role,task_family,restoration_mode,cross_harness_reuse,source,created_at
               FROM usage_ledger WHERE workspace_id=?1 AND (?2 IS NULL OR session_id=?2) ORDER BY id";
    query_with_params(db, sql, params![workspace_id, session_id], |row| {
        Ok(UsageLedgerRow {
            id: row.get(0)?,
            workspace_id: row.get(1)?,
            session_id: row.get(2)?,
            turn_id: row.get(3)?,
            input_tokens: row.get(4)?,
            output_tokens: row.get(5)?,
            cache_read_tokens: row.get(6)?,
            cache_write_tokens: row.get(7)?,
            uncached_input_tokens: row.get(8)?,
            context_percent: row.get(9)?,
            capability_units: row.get(10)?,
            runtime_ms: row.get(11)?,
            cost_microusd: row.get(12)?,
            cost_source: row.get(13)?,
            stable_prefix_id: row.get(14)?,
            stable_prefix_hash: row.get(15)?,
            prompt_schema_version: row.get(16)?,
            prefix_token_estimate: row.get(17)?,
            harness: row.get(18)?,
            model: row.get(19)?,
            role: row.get(20)?,
            task_family: row.get(21)?,
            restoration_mode: row.get(22)?,
            cross_harness_reuse: row.get(23)?,
            source: row.get(24)?,
            created_at: row.get(25)?,
        })
    })
}

pub fn record_prompt_compilation(
    db: &Connection,
    record: &PromptCompilationRecord,
) -> Result<i64, BridgeError> {
    db.execute(
        "INSERT INTO prompt_compilations(session_id,turn_id,prefix_id,prefix_hash,schema_version,prefix_bytes,prefix_token_estimate,harness,model,role,task_family,restoration_mode,cross_harness_reuse,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        params![record.session_id,record.turn_id,record.prefix_id,record.prefix_hash,record.schema_version,record.prefix_bytes,record.prefix_token_estimate,record.harness,record.model,record.role,record.task_family,record.restoration_mode,record.cross_harness_reuse,record.created_at],
    )?;
    Ok(db.last_insert_rowid())
}

pub fn latest_prompt_compilation(
    db: &Connection,
    session_id: &str,
) -> Result<Option<PromptCompilationRecord>, BridgeError> {
    Ok(db.query_row(
        "SELECT id,session_id,turn_id,prefix_id,prefix_hash,schema_version,prefix_bytes,prefix_token_estimate,harness,model,role,task_family,restoration_mode,cross_harness_reuse,created_at
         FROM prompt_compilations WHERE session_id=?1 ORDER BY id DESC LIMIT 1",
        params![session_id],
        |row| Ok(PromptCompilationRecord { id:row.get(0)?, session_id:row.get(1)?, turn_id:row.get(2)?, prefix_id:row.get(3)?, prefix_hash:row.get(4)?, schema_version:row.get(5)?, prefix_bytes:row.get(6)?, prefix_token_estimate:row.get(7)?, harness:row.get(8)?, model:row.get(9)?, role:row.get(10)?, task_family:row.get(11)?, restoration_mode:row.get(12)?, cross_harness_reuse:row.get(13)?, created_at:row.get(14)? }),
    ).optional()?)
}

pub fn bind_latest_prompt_compilation_to_turn(
    db: &Connection,
    session_id: &str,
    turn_id: &str,
) -> Result<bool, BridgeError> {
    Ok(db.execute(
        "UPDATE prompt_compilations SET turn_id=?2 WHERE id=(SELECT id FROM prompt_compilations WHERE session_id=?1 AND turn_id IS NULL ORDER BY id DESC LIMIT 1)",
        params![session_id, turn_id],
    )? == 1)
}

pub fn prompt_compilation_for_turn(
    db: &Connection,
    session_id: &str,
    turn_id: &str,
) -> Result<Option<PromptCompilationRecord>, BridgeError> {
    Ok(db.query_row(
        "SELECT id,session_id,turn_id,prefix_id,prefix_hash,schema_version,prefix_bytes,prefix_token_estimate,harness,model,role,task_family,restoration_mode,cross_harness_reuse,created_at
         FROM prompt_compilations WHERE session_id=?1 AND turn_id=?2 ORDER BY id DESC LIMIT 1",
        params![session_id, turn_id],
        |row| Ok(PromptCompilationRecord { id:row.get(0)?, session_id:row.get(1)?, turn_id:row.get(2)?, prefix_id:row.get(3)?, prefix_hash:row.get(4)?, schema_version:row.get(5)?, prefix_bytes:row.get(6)?, prefix_token_estimate:row.get(7)?, harness:row.get(8)?, model:row.get(9)?, role:row.get(10)?, task_family:row.get(11)?, restoration_mode:row.get(12)?, cross_harness_reuse:row.get(13)?, created_at:row.get(14)? }),
    ).optional()?)
}

pub fn delete_prompt_compilation(db: &Connection, id: i64) -> Result<bool, BridgeError> {
    Ok(db.execute("DELETE FROM prompt_compilations WHERE id=?1", params![id])? == 1)
}

fn query_with_params<T, P, F>(
    db: &Connection,
    sql: &str,
    params: P,
    mut map: F,
) -> Result<Vec<T>, BridgeError>
where
    P: rusqlite::Params,
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    let mut statement = db.prepare(sql)?;
    let rows = statement.query_map(params, |row| map(row))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

fn parse_json_column(row: &rusqlite::Row<'_>, index: usize) -> serde_json::Value {
    row.get::<_, String>(index)
        .ok()
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or(serde_json::Value::Null)
}

pub fn session_event(
    db: &Connection,
    session_id: &str,
    event: &crate::agent::NormalizedEvent,
    provider_meta: &serde_json::Value,
) -> Result<AgentEvent, BridgeError> {
    event.validate().map_err(BridgeError::Invalid)?;
    let transaction = db.unchecked_transaction()?;
    let stored = session_event_in_transaction(&transaction, session_id, event, provider_meta)?;
    transaction.commit()?;
    Ok(stored)
}

/// Append a durable agent event inside a caller-owned transaction.
///
/// Callers that update related session state use this helper so the state
/// change, audit records, and durable notification history commit together.
pub(crate) fn session_event_in_transaction(
    transaction: &Transaction<'_>,
    session_id: &str,
    event: &crate::agent::NormalizedEvent,
    provider_meta: &serde_json::Value,
) -> Result<AgentEvent, BridgeError> {
    event.validate().map_err(BridgeError::Invalid)?;
    let parent_entry_id: Option<String> = transaction
        .query_row(
            "SELECT active_entry_id FROM session_heads WHERE session_id=?1",
            params![session_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let trace_id: String = transaction
        .query_row(
            "SELECT COALESCE(trace_id,id) FROM sessions WHERE id=?1",
            params![session_id],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| session_id.to_owned());
    let payload = serde_json::json!({
        "protocolVersion": 1,
        "itemId": event.item_id,
        "role": event.role,
        "status": event.status,
        "title": event.title,
        "text": event.text,
        "data": event.data,
        "providerMeta": provider_meta,
        "traceId": trace_id,
    });
    let mut final_kind = event.kind.as_str();
    if final_kind == "message.completed" {
        final_kind = if event.role.as_deref() == Some("user") {
            "user.message"
        } else {
            "assistant.message"
        };
    } else if final_kind == "tool.started"
        || final_kind == "tool.completed"
        || final_kind == "approval.requested"
        || final_kind == "approval.resolved"
        || final_kind == "delegation.requested"
        || final_kind == "delegation.approved"
        || final_kind == "delegation.rejected"
        || final_kind == "worker.result"
    {
        // Keep as is, it maps directly.
    } else if final_kind.ends_with(".delta")
        || final_kind.ends_with(".progress")
        || final_kind == "turn.started"
        || final_kind == "turn.completed"
        || final_kind == "usage.updated"
        || final_kind == "plan.updated"
    {
        // Do not store transient or internal events in the immutable forest.
        return Ok(AgentEvent {
            id: 0,
            session_id: session_id.into(),
            sequence: 0,
            protocol_version: 1,
            kind: event.kind.clone(),
            item_id: event.item_id.clone(),
            role: event.role.clone(),
            status: event.status.clone(),
            title: event.title.clone(),
            text: event.text.clone(),
            data: event.data.clone(),
            provider_meta: provider_meta.clone(),
            created_at: Utc::now().to_rfc3339(),
        });
    }

    let entry = append_session_entry_tx(
        transaction,
        session_id,
        parent_entry_id.as_deref(),
        final_kind,
        &payload,
        event.item_id.as_deref(),
        "eligible",
        None,
    )?;
    Ok(AgentEvent {
        id: entry.sequence,
        session_id: session_id.into(),
        sequence: entry.sequence,
        protocol_version: 1,
        kind: event.kind.clone(),
        item_id: event.item_id.clone(),
        role: event.role.clone(),
        status: event.status.clone(),
        title: event.title.clone(),
        text: event.text.clone(),
        data: event.data.clone(),
        provider_meta: provider_meta.clone(),
        created_at: entry.created_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_harness_column_round_trips_every_shape() {
        // The column is TEXT and always has been, so opening the identifier
        // needs no schema migration — only an honest reading of what is there.
        for stored in [
            "claude",
            "codex",
            "opencode",
            "shell",
            "gemini",
            "github-copilot-cli",
            "mistral-vibe",
        ] {
            let parsed = harness(stored);
            assert_eq!(
                harness_name(&parsed),
                stored,
                "{stored:?} did not survive the column round trip"
            );
        }
    }

    #[test]
    fn an_unreadable_harness_row_never_becomes_a_runnable_harness() {
        // Regression for `_ => Harness::Shell`: an id this build cannot
        // interpret used to load as Shell, which is a real, runnable harness.
        for stored in ["", "SHELL", "Claude", "acp:gemini", "gem ini"] {
            let parsed = harness(stored);
            assert_eq!(parsed, Harness::Unknown(stored.to_owned()), "{stored:?}");
            assert_ne!(parsed, Harness::Shell, "{stored:?} was read as Shell");
            assert_eq!(harness_name(&parsed), stored, "{stored:?} lost its raw id");
        }
    }

    #[test]
    fn a_session_of_an_uninstalled_harness_still_loads_with_its_history() {
        // The acceptance criterion in miniature: uninstalling an agent must
        // not make its sessions vanish or be re-attributed to another harness.
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("bridge.db")).unwrap();
        seed_workspace(&db);
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) \
             VALUES('s-acp','w','gemini','Gemini','ready','estimated')",
            [],
        )
        .unwrap();
        append_session_entry(
            &db,
            "s-acp",
            None,
            "assistant.message",
            &json!({"text":"history that must survive"}),
            None,
            "eligible",
            Some(1),
        )
        .unwrap();

        let state = state(&db).unwrap();
        let session = state
            .sessions
            .iter()
            .find(|session| session.id == "s-acp")
            .expect("the session still loads");
        assert_eq!(session.harness, Harness::from_stored("gemini"));
        assert_eq!(
            serde_json::to_value(&session.harness).unwrap(),
            json!("gemini"),
            "it renders under its own id"
        );

        let events = session_events_after(&db, "s-acp", 0, 10).unwrap();
        assert_eq!(events.len(), 1, "replay still returns its history");
        assert_eq!(events[0].text.as_deref(), Some("history that must survive"));
    }

    fn seed_workspace(db: &Connection) {
        db.execute(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/demo','now')",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Kyoto','Task','bridge/task','/tmp/w','idle','now')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,provider_session_id) VALUES('s','w','codex','Codex','working','reported','native-s')", []).unwrap();
    }

    fn create_legacy_fixture(path: &Path) {
        let db = Connection::open(path).unwrap();
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
            CREATE TABLE projects (id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE, created_at TEXT NOT NULL);
            CREATE TABLE workspaces (id TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id), city TEXT NOT NULL, title TEXT NOT NULL, branch TEXT NOT NULL, path TEXT NOT NULL UNIQUE, status TEXT NOT NULL, dirty_files INTEGER NOT NULL DEFAULT 0, additions INTEGER NOT NULL DEFAULT 0, deletions INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL);
            CREATE TABLE sessions (id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL REFERENCES workspaces(id), harness TEXT NOT NULL, label TEXT NOT NULL, status TEXT NOT NULL, started_at TEXT, ended_at TEXT, context_percent INTEGER, usage_percent INTEGER, metric_source TEXT NOT NULL DEFAULT 'estimated');
            CREATE TABLE events (id INTEGER PRIMARY KEY AUTOINCREMENT, source TEXT NOT NULL, kind TEXT NOT NULL, entity_id TEXT NOT NULL, body TEXT NOT NULL, created_at TEXT NOT NULL);
            CREATE TABLE agent_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL REFERENCES sessions(id),
                sequence INTEGER NOT NULL,
                protocol_version INTEGER NOT NULL DEFAULT 1,
                kind TEXT NOT NULL,
                item_id TEXT,
                role TEXT,
                status TEXT,
                title TEXT,
                text TEXT,
                data TEXT NOT NULL DEFAULT '{}',
                provider_meta TEXT NOT NULL DEFAULT '{}',
                created_at TEXT NOT NULL,
                UNIQUE(session_id, sequence)
            );
            CREATE INDEX idx_agent_events_session ON agent_events(session_id, sequence);
            ALTER TABLE sessions ADD COLUMN provider_session_id TEXT;
            ALTER TABLE sessions ADD COLUMN active_turn_id TEXT;
            ALTER TABLE sessions ADD COLUMN model TEXT;
            ALTER TABLE sessions ADD COLUMN effort TEXT;
            ALTER TABLE sessions ADD COLUMN parent_session_id TEXT;
            ALTER TABLE sessions ADD COLUMN depth INTEGER;",
        )
        .unwrap();
        seed_workspace(&db);
        db.execute(
            "INSERT INTO agent_events(session_id,sequence,kind,item_id,role,status,title,text,data,provider_meta,created_at)
             VALUES('s',1,'assistant.message','m1','assistant','inProgress','First','hello','{\"delta\":\"hello\"}','{\"provider\":\"codex\",\"rawId\":1}','t1')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO agent_events(session_id,sequence,kind,item_id,role,status,title,text,data,provider_meta,created_at)
             VALUES('s',2,'assistant.message','m2','assistant','completed','Second','world','{\"delta\":\"world\"}','{\"provider\":\"codex\",\"rawId\":2}','t2')",
            [],
        )
        .unwrap();
    }

    fn schema_signature(db: &Connection) -> Vec<String> {
        let mut objects = {
            let mut statement = db
                .prepare(
                    "SELECT type,name,tbl_name,CASE WHEN type='index' THEN COALESCE(sql,'') ELSE '' END FROM sqlite_master
                     WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name",
                )
                .unwrap();
            let rows = statement
                .query_map([], |row| {
                    Ok(format!(
                        "{}:{}:{}:{}",
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?
                    ))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            rows
        };
        let tables = [
            "schema_version",
            "projects",
            "workspaces",
            "sessions",
            "events",
            "agent_events",
            "session_entries",
            "session_heads",
            "task_knowledge",
            "worker_leases",
            "worker_runtime",
            "delegation_receipts",
            "worker_queue",
            "usage_ledger",
            "routing_evaluations",
            "learning_trigger_events",
        ];
        for table in tables {
            let mut statement = db.prepare(&format!("PRAGMA table_info({table})")).unwrap();
            let columns = statement
                .query_map([], |row| {
                    Ok(format!(
                        "{}:{}:{}:{}:{:?}:{}",
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, i64>(5)?
                    ))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            objects.push(format!("columns:{table}:{}", columns.join("|")));
        }
        objects
    }

    fn migration_versions(db: &Connection) -> Vec<i64> {
        let mut statement = db
            .prepare("SELECT version FROM schema_version ORDER BY version")
            .unwrap();
        statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    fn backup_paths(dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("bridge.db.backup-"))
            })
            .collect()
    }

    #[test]
    fn persists_and_replays_ordered_events() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("bridge.db")).unwrap();
        db.execute(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/demo','now')",
            [],
        )
        .unwrap();
        event(&db, "git", "first", "p", "one").unwrap();
        event(&db, "git", "second", "p", "two").unwrap();
        let snapshot = state(&db).unwrap();
        assert_eq!(snapshot.projects.len(), 1);
        assert_eq!(snapshot.events[0].kind, "second");
        assert_eq!(snapshot.events[1].kind, "first");
    }

    #[test]
    fn telemetry_batch_uses_an_independent_writer_and_failure_cannot_rollback_history() {
        let dir = tempfile::tempdir().unwrap();
        let primary = open(&dir.path().join("bridge.db")).unwrap();
        let telemetry = open_telemetry(&dir.path().join("bridge-telemetry.db")).unwrap();
        seed_workspace(&primary);
        let event = crate::agent::NormalizedEvent {
            kind: "message.completed".into(),
            item_id: Some("m1".into()),
            role: Some("assistant".into()),
            status: Some("completed".into()),
            title: None,
            text: Some("durable".into()),
            data: json!({}),
        };
        let committed = session_event(&primary, "s", &event, &json!({"adapter":"codex"})).unwrap();
        let write_lock = primary.unchecked_transaction().unwrap();
        write_lock
            .execute("UPDATE sessions SET label='locked' WHERE id='s'", [])
            .unwrap();
        let span = telemetry_span("trace", "s", "codex", &event, &committed.created_at);
        assert_eq!(
            append_telemetry_batch(&telemetry, std::slice::from_ref(&span)).unwrap(),
            1
        );
        write_lock.rollback().unwrap();
        assert_eq!(
            telemetry
                .query_row("SELECT COUNT(*) FROM telemetry_spans", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            primary
                .query_row("SELECT COUNT(*) FROM session_entries", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            primary
                .query_row("SELECT COUNT(*) FROM telemetry_spans", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );

        telemetry.execute("DROP TABLE telemetry_spans", []).unwrap();
        assert!(append_telemetry_batch(&telemetry, &[span]).is_err());
        assert_eq!(
            primary
                .query_row("SELECT COUNT(*) FROM session_entries", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn history_snapshot_is_consistent_and_checksum_detects_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let primary_path = dir.path().join("bridge.db");
        let primary = open(&primary_path).unwrap();
        event(&primary, "test", "history.saved", "entity", "durable").unwrap();
        let readonly = Connection::open_with_flags(
            &primary_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .unwrap();
        let (snapshot, manifest) =
            export_history_snapshot(&readonly, &dir.path().join("snapshots")).unwrap();
        assert!(verify_history_snapshot(&snapshot, &manifest).unwrap());
        let snapshot_db = Connection::open(&snapshot).unwrap();
        assert_eq!(
            snapshot_db
                .query_row(
                    "SELECT body FROM events WHERE kind='history.saved'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "durable"
        );
        drop(snapshot_db);

        let mut bytes = std::fs::read(&snapshot).unwrap();
        bytes[0] ^= 0xff;
        std::fs::write(&snapshot, bytes).unwrap();
        assert!(!verify_history_snapshot(&snapshot, &manifest).unwrap());
    }

    #[test]
    fn migrates_current_schema_fixture_idempotently_and_creates_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        create_legacy_fixture(&path);
        let db = open(&path).unwrap();
        assert_eq!(
            migration_versions(&db),
            (1..=LATEST_SCHEMA_VERSION).collect::<Vec<_>>(),
            "a legacy fixture must land on the current schema"
        );
        for table in [
            "model_profiles",
            "routing_policies",
            "routing_evaluations",
            "learning_jobs",
            "learning_triggers",
            "learning_job_runs",
            "learning_trigger_events",
            "routing_policy_promotions",
            "prompt_compilations",
            "learning_scope_cursors",
            "session_entry_fts",
        ] {
            assert!(
                db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                    params![table],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap(),
                "missing migration-15 table {table}"
            );
        }
        for (table, column) in [
            ("usage_ledger", "cost_microusd"),
            ("usage_ledger", "uncached_input_tokens"),
            ("usage_ledger", "stable_prefix_hash"),
            ("model_profiles", "profile_id"),
            ("router_decisions", "policy_version"),
            ("router_decisions", "trace_id"),
            ("router_decisions", "catalog_snapshot"),
            ("router_outcomes", "success_state"),
            ("router_outcomes", "confidence_bps"),
            ("learning_job_runs", "lease_expires_at"),
            ("learning_jobs", "last_evidence_boundary"),
            ("learning_jobs", "run_budget_tokens"),
            ("routing_policies", "learning_scope"),
            ("learning_job_runs", "learning_scope"),
        ] {
            let exists = db
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap()
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
                .iter()
                .any(|name| name == column);
            assert!(exists, "missing migration-15 column {table}.{column}");
        }
        {
            let transaction = db.unchecked_transaction().unwrap();
            migration_15_role_profiles_and_learning_jobs(&transaction).unwrap();
            transaction.commit().unwrap();
        }
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM routing_policies WHERE status='active'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(db.query_row("SELECT COUNT(*) FROM learning_triggers WHERE kind IN ('manual','in_app') AND registration_id='built-in'", [], |row| row.get::<_, i64>(0)).unwrap(), 2);
        // Migration 22 adds the backend binding without inventing one. A row
        // that predates it stays readable and reads as unbound — a guessed
        // backend would be fabricated provenance for a session that never had
        // any, and the resume path treats null as "not recorded", not as
        // "changed".
        let (backend, version, installation): (
            Option<String>,
            Option<String>,
            Option<String>,
        ) = db
            .query_row(
                "SELECT backend_id,backend_version,backend_installation_id FROM sessions WHERE id='s'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!((backend, version, installation), (None, None, None));
        assert!(db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='backend_change_authorizations')",
                [],
                |row| row.get::<_, bool>(0),
            )
            .unwrap());
        {
            let transaction = db.unchecked_transaction().unwrap();
            migration_22_session_backend_binding(&transaction).unwrap();
            transaction.commit().unwrap();
        }

        // Legacy agent_events were backfilled into the immutable forest.
        assert_eq!(session_entries(&db, "s").unwrap().len(), 2);
        drop(db);
        let backups = backup_paths(dir.path());
        assert_eq!(backups.len(), 1);
        let backup = Connection::open(&backups[0]).unwrap();
        assert_eq!(
            backup
                .query_row("SELECT COUNT(*) FROM agent_events", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
        drop(backup);
        let db = open(&path).unwrap();
        assert_eq!(
            migration_versions(&db),
            (1..=LATEST_SCHEMA_VERSION).collect::<Vec<_>>(),
            "a legacy fixture must land on the current schema"
        );
        assert_eq!(backup_paths(dir.path()).len(), 1);
    }

    #[test]
    fn upgraded_and_current_databases_have_identical_cache_telemetry_columns() {
        fn columns(db: &Connection, table: &str) -> Vec<String> {
            let mut values = db
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap()
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            values.sort();
            values
        }

        let current = open(Path::new(":memory:")).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let legacy_path = dir.path().join("legacy.db");
        create_legacy_fixture(&legacy_path);
        let upgraded = open(&legacy_path).unwrap();
        assert_eq!(
            columns(&current, "usage_ledger"),
            columns(&upgraded, "usage_ledger")
        );
        assert_eq!(
            columns(&current, "prompt_compilations"),
            columns(&upgraded, "prompt_compilations")
        );
        for required in [
            "uncached_input_tokens",
            "stable_prefix_id",
            "stable_prefix_hash",
            "prompt_schema_version",
            "prefix_token_estimate",
            "cross_harness_reuse",
        ] {
            assert!(columns(&upgraded, "usage_ledger")
                .iter()
                .any(|column| column == required));
        }
    }

    #[test]
    fn migration_16_repairs_databases_created_by_early_migration_15() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        let db = open(&path).unwrap();
        db.execute_batch(
            "DROP INDEX idx_model_profiles_id;
             ALTER TABLE model_profiles DROP COLUMN profile_id;
             ALTER TABLE learning_jobs DROP COLUMN last_evidence_boundary;
             DROP TABLE configuration_entries;
             DELETE FROM schema_version WHERE version >= 16;",
        )
        .unwrap();
        drop(db);

        let db = open(&path).unwrap();
        assert_eq!(current_schema_version(&db).unwrap(), LATEST_SCHEMA_VERSION);
        for (table, column) in [
            ("model_profiles", "profile_id"),
            ("learning_jobs", "last_evidence_boundary"),
        ] {
            let exists = db
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap()
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
                .iter()
                .any(|name| name == column);
            assert!(exists, "migration 16 did not restore {table}.{column}");
        }
        db.execute(
            "INSERT INTO model_profiles(version,profile_id,purpose,canonical_role,provider,model,effort,created_at)
             VALUES(1,'standard_orchestrator','standard_orchestrator','orchestrator','codex','test-model','medium','now')",
            [],
        )
        .unwrap();
    }

    #[test]
    fn migrations_19_and_20_repair_the_true_early_v15_shape() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        let db = open(&path).unwrap();
        let late_columns = [
            ("router_decisions", "task_fingerprint"),
            ("router_decisions", "trace_id"),
            ("router_decisions", "repository_revision"),
            ("router_decisions", "profile_version"),
            ("router_decisions", "profile_purpose"),
            ("router_decisions", "policy_version"),
            ("router_decisions", "catalog_snapshot"),
            ("router_decisions", "selection_reason"),
            ("router_decisions", "actual_provider"),
            ("router_decisions", "actual_model"),
            ("router_decisions", "actual_effort"),
            ("router_outcomes", "success_state"),
            ("router_outcomes", "acceptance_state"),
            ("router_outcomes", "cost_microusd"),
            ("router_outcomes", "cost_source"),
            ("router_outcomes", "confidence_bps"),
            ("router_outcomes", "edit_count"),
            ("router_outcomes", "override_signal"),
            ("router_outcomes", "total_tokens"),
            ("router_outcomes", "latency_source"),
            ("learning_jobs", "run_budget_tokens"),
            ("learning_triggers", "auth_digest"),
            ("learning_triggers", "expires_at"),
            ("learning_triggers", "experimental"),
            ("learning_triggers", "updated_at"),
            ("learning_job_runs", "lease_owner"),
            ("learning_job_runs", "lease_expires_at"),
            ("learning_job_runs", "snapshot_frozen_at"),
            ("learning_job_runs", "evaluated_spend_microusd"),
            ("learning_job_runs", "evaluated_tokens"),
            ("learning_job_runs", "replay_passed"),
            ("learning_job_runs", "promotion_status"),
            ("routing_policies", "rollback_of"),
            ("routing_policies", "replay_report"),
            ("routing_policies", "promoted_at"),
            ("routing_policies", "activation_boundary"),
        ];
        for (table, column) in late_columns {
            db.execute_batch(&format!("ALTER TABLE {table} DROP COLUMN {column}"))
                .unwrap();
        }
        db.execute_batch(
            "DROP TABLE routing_policy_promotions;
             DROP TABLE routing_evaluations;
             DROP TABLE learning_trigger_events;
             DROP TABLE IF EXISTS learning_scope_cursors;
             DROP INDEX idx_routing_policy_active;
             ALTER TABLE routing_policies DROP COLUMN learning_scope;
             ALTER TABLE learning_job_runs DROP COLUMN learning_scope;
             CREATE UNIQUE INDEX idx_routing_policy_active
                ON routing_policies(status) WHERE status='active';
             CREATE TABLE routing_evaluations (
                id TEXT PRIMARY KEY,
                run_id TEXT NOT NULL,
                evaluator_kind TEXT NOT NULL,
                evaluator_version TEXT NOT NULL,
                score_bps INTEGER,
                confidence_bps INTEGER,
                evidence_entry_ids TEXT NOT NULL DEFAULT '[]',
                created_at TEXT NOT NULL
             );
             CREATE TABLE learning_trigger_events (
                id TEXT PRIMARY KEY,
                run_id TEXT NOT NULL REFERENCES learning_job_runs(id) ON DELETE CASCADE,
                trigger_kind TEXT NOT NULL,
                result TEXT NOT NULL,
                created_at TEXT NOT NULL
             );
             DELETE FROM schema_version WHERE version >= 19;",
        )
        .unwrap();
        drop(db);

        let db = open(&path).unwrap();
        assert_eq!(current_schema_version(&db).unwrap(), LATEST_SCHEMA_VERSION);
        for (table, column) in late_columns {
            let exists = db
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap()
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
                .iter()
                .any(|name| name == column);
            assert!(exists, "migration 19 did not restore {table}.{column}");
        }
        assert!(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='routing_policy_promotions')",
            [], |row| row.get::<_, bool>(0),
        ).unwrap());
        let legacy_run_id: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('routing_evaluations') WHERE name='run_id')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!legacy_run_id);
        db.execute(
            "INSERT INTO routing_evaluations(id,evaluator_kind,evaluator_version,created_at)
             VALUES('post-repair-eval','deterministic','test-v1','now')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO learning_trigger_events(id,run_id,trigger_kind,result,created_at)
             VALUES('post-repair-trigger',NULL,'manual','accepted','now')",
            [],
        )
        .unwrap();
        let trigger_delete_action: String = db.query_row(
            "SELECT on_delete FROM pragma_foreign_key_list('learning_trigger_events') WHERE \"from\"='run_id'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(trigger_delete_action, "SET NULL");
        let active_index_sql: String = db.query_row(
            "SELECT sql FROM sqlite_master WHERE type='index' AND name='idx_routing_policy_active'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert!(active_index_sql.contains("status IN ('active','canary')"));
        assert!(
            active_index_sql.contains("learning_scope"),
            "the live-policy unique index must be per learning_scope: {active_index_sql}"
        );
        for (table, column) in [
            ("routing_policies", "learning_scope"),
            ("learning_job_runs", "learning_scope"),
            ("routing_policy_promotions", "learning_scope"),
        ] {
            let exists = db
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap()
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
                .iter()
                .any(|name| name == column);
            assert!(exists, "migration 26 did not restore {table}.{column}");
        }
        assert!(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='learning_scope_cursors')",
            [], |row| row.get::<_, bool>(0),
        ).unwrap());
    }

    #[test]
    fn learning_scope_migration_backfills_legacy_global_and_allows_one_live_policy_per_scope() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("bridge.db")).unwrap();
        let scope: String = db
            .query_row(
                "SELECT learning_scope FROM routing_policies WHERE version=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(scope, "legacy:global");
        db.execute(
            "INSERT INTO routing_policies(version,status,learning_scope,weights,thresholds,created_reason,created_at)
             VALUES(2,'active','workspace:a','{}','{}','test','now')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO routing_policies(version,status,learning_scope,weights,thresholds,created_reason,created_at)
             VALUES(3,'active','workspace:b','{}','{}','test','now')",
            [],
        )
        .unwrap();
        let error = db
            .execute(
                "INSERT INTO routing_policies(version,status,learning_scope,weights,thresholds,created_reason,created_at)
                 VALUES(4,'canary','workspace:a','{}','{}','test','now')",
                [],
            )
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("UNIQUE") || error.contains("unique"),
            "{error}"
        );
    }

    #[test]
    fn additive_repair_skips_tables_that_do_not_exist_yet() {
        let mut db = Connection::open_in_memory().unwrap();
        let transaction = db.transaction().unwrap();
        add_column_if_missing(&transaction, "future_table", "future_column", "TEXT").unwrap();
        assert!(!table_exists(&transaction, "future_table").unwrap());
        transaction.commit().unwrap();
    }

    #[test]
    fn fresh_and_upgraded_databases_have_identical_schema() {
        let fresh_dir = tempfile::tempdir().unwrap();
        let upgraded_dir = tempfile::tempdir().unwrap();
        let fresh = open(&fresh_dir.path().join("bridge.db")).unwrap();
        let upgraded_path = upgraded_dir.path().join("bridge.db");
        create_legacy_fixture(&upgraded_path);
        let upgraded = open(&upgraded_path).unwrap();
        assert_eq!(schema_signature(&fresh), schema_signature(&upgraded));
        assert_eq!(migration_versions(&fresh), migration_versions(&upgraded));
    }

    #[test]
    fn capability_tier_migration_preserves_actual_models_and_is_idempotent() {
        let mut db = Connection::open(":memory:").unwrap();
        {
            let transaction = db.transaction().unwrap();
            migration_1_current_schema(&transaction).unwrap();
            transaction
                .execute("INSERT INTO schema_version VALUES(1,'now')", [])
                .unwrap();
            transaction.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/tier-migration','now')", []).unwrap();
            transaction.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Task','bridge/task','/tmp/tier-workspace','idle','now')", []).unwrap();
            transaction.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,model) VALUES('s','w','codex','Worker','ready','reported','runtime-model')", []).unwrap();
            migration_2_session_forest(&transaction).unwrap();
            transaction
                .execute("INSERT INTO schema_version VALUES(2,'now')", [])
                .unwrap();
            transaction.commit().unwrap();
        }
        {
            let transaction = db.transaction().unwrap();
            migration_3_capability_tiers(&transaction).unwrap();
            migration_3_capability_tiers(&transaction).unwrap();
            transaction
                .execute("INSERT INTO schema_version VALUES(3,'now')", [])
                .unwrap();
            transaction.commit().unwrap();
        }
        let (model, tier): (String, Option<String>) = db
            .query_row(
                "SELECT model,requested_tier FROM sessions WHERE id='s'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(model, "runtime-model");
        assert_eq!(tier, None);
        let tier_columns: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('sessions') WHERE name='requested_tier'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tier_columns, 1);
    }

    #[test]
    fn continuation_fidelity_migration_derives_conservative_history_markers() {
        let mut db = open(Path::new(":memory:")).unwrap();
        db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/fidelity','now')", []).unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Task','bridge/task','/tmp/fidelity-w','idle','now')", []).unwrap();
        for (id, parent) in [
            ("root", None),
            ("boundary", Some("root")),
            ("mid", Some("root")),
            ("resumed", Some("root")),
        ] {
            db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id,continuation_fidelity) VALUES(?1,'w','codex','Session','stopped','reported',?2,'native')", params![id,parent]).unwrap();
        }
        for (id, mode) in [
            ("root", "fresh"),
            ("boundary", "checkpoint_restored"),
            ("mid", "fresh"),
            ("resumed", "native"),
        ] {
            db.execute("INSERT INTO session_heads(session_id,restoration_mode,updated_at) VALUES(?1,?2,'now')", params![id,mode]).unwrap();
        }
        let transaction = db.transaction().unwrap();
        migration_10_continuation_fidelity(&transaction).unwrap();
        transaction.commit().unwrap();
        let values = query(&db, "SELECT continuation_fidelity FROM sessions ORDER BY CASE id WHEN 'root' THEN 1 WHEN 'boundary' THEN 2 WHEN 'mid' THEN 3 ELSE 4 END", |row| row.get::<_,String>(0)).unwrap();
        assert_eq!(
            values,
            vec![
                "native",
                "projected_at_boundary",
                "projected_mid_turn",
                "native"
            ]
        );
    }

    #[test]
    fn human_blocked_queue_migration_preserves_existing_rows() {
        let mut db = Connection::open(":memory:").unwrap();
        db.execute_batch("CREATE TABLE worker_queue(id TEXT PRIMARY KEY,queue_status TEXT NOT NULL,expires_at TEXT); INSERT INTO worker_queue VALUES('q','queued','2099-01-01T00:00:00+00:00');").unwrap();
        let transaction = db.transaction().unwrap();
        migration_11_human_blocked_queue(&transaction).unwrap();
        transaction.commit().unwrap();
        let row = db
            .query_row(
                "SELECT queue_status,expires_at,blocked_at FROM worker_queue WHERE id='q'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            row,
            ("queued".into(), "2099-01-01T00:00:00+00:00".into(), None)
        );
    }

    #[test]
    fn adapter_process_claim_migration_preserves_existing_sessions() {
        let mut db = Connection::open(":memory:").unwrap();
        db.execute_batch("CREATE TABLE sessions(id TEXT PRIMARY KEY,status TEXT NOT NULL); INSERT INTO sessions VALUES('s','working');").unwrap();
        let transaction = db.transaction().unwrap();
        migration_12_adapter_process_claims(&transaction).unwrap();
        transaction.commit().unwrap();
        let row = db
            .query_row(
                "SELECT status,adapter_pid,adapter_process_identity FROM sessions WHERE id='s'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(row, ("working".into(), None, None));
    }

    #[test]
    fn resume_eligibility_migration_preserves_existing_restoration_state() {
        let mut db = Connection::open(":memory:").unwrap();
        {
            let transaction = db.transaction().unwrap();
            migration_1_current_schema(&transaction).unwrap();
            migration_2_session_forest(&transaction).unwrap();
            migration_3_capability_tiers(&transaction).unwrap();
            transaction.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/resume-migration','now')", []).unwrap();
            transaction.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Task','bridge/task','/tmp/resume-workspace','stopped','now')", []).unwrap();
            transaction.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,provider_session_id) VALUES('s','w','codex','Worker','stopped','reported','native-s')", []).unwrap();
            transaction.execute("INSERT INTO session_heads(session_id,native_provider_session_id,restoration_mode,updated_at) VALUES('s','native-s','native','now')", []).unwrap();
            transaction.commit().unwrap();
        }

        let transaction = db.transaction().unwrap();
        migration_4_resume_eligibility(&transaction).unwrap();
        migration_4_resume_eligibility(&transaction).unwrap();
        transaction.commit().unwrap();

        let head = session_head(&db, "s").unwrap().unwrap();
        assert_eq!(head.native_provider_session_id.as_deref(), Some("native-s"));
        assert_eq!(head.restoration_mode, RestorationMode::Native);
        assert_eq!(head.resume_eligibility, ResumeEligibility::Fresh);
    }

    /// Inserts one run and returns its id, satisfying every NOT NULL column.
    fn insert_work_run(db: &Connection, id: &str) {
        db.execute(
            "INSERT INTO work_brief_runs(id,trigger_kind,status,max_wall_seconds,max_turns,
                 max_tool_calls,started_at)
             VALUES(?1,'manual','running',600,12,24,'2026-08-19T09:00:00+00:00')",
            params![id],
        )
        .unwrap();
    }

    #[test]
    fn migration_24_adds_the_work_tables_to_an_existing_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        create_legacy_fixture(&path);
        let db = open(&path).unwrap();
        for table in [
            "work_brief_runs",
            "work_brief_sources",
            "work_evidence",
            "work_tasks",
            "work_fact_cache",
        ] {
            assert!(
                db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                    params![table],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap(),
                "missing migration-24 table {table}"
            );
        }
        // The upgrade is additive: the fixture's own rows are still there, which
        // is what "readable by the previous binary" rests on.
        let sessions: i64 = db
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(sessions, 1);
        let entries: i64 = db
            .query_row("SELECT COUNT(*) FROM session_entries", [], |row| row.get(0))
            .unwrap();
        assert_eq!(entries, 2, "backfilled entries survive the Work migration");
    }

    #[test]
    fn work_tables_declare_their_unique_constraints() {
        let db = open(Path::new(":memory:")).unwrap();
        insert_work_run(&db, "run-1");
        insert_work_run(&db, "run-2");

        db.execute(
            "INSERT INTO work_brief_sources(run_id,connector_instance_id,connector_family,status)
             VALUES('run-1','github:acme','github','eligible')",
            [],
        )
        .unwrap();
        assert!(
            db.execute(
                "INSERT INTO work_brief_sources(run_id,connector_instance_id,connector_family,status)
                 VALUES('run-1','github:acme','github','failed')",
                [],
            )
            .is_err(),
            "one row per run and connector instance, or coverage can say two things at once"
        );
        db.execute(
            "INSERT INTO work_brief_sources(run_id,connector_instance_id,connector_family,status)
             VALUES('run-2','github:acme','github','eligible')",
            [],
        )
        .expect("the same connector in a different run is a different row");

        db.execute(
            "INSERT INTO work_evidence(run_id,evidence_ref,tool_call_id,connector_instance_id,
                 canonical_resource_id,source_kind,tool_definition_digest,result_digest,succeeded,observed_at)
             VALUES('run-1','ev-1','call-1','github:acme','acme/bridge#204','github_issue',
                    'sha256:tool','sha256:result',1,'2026-08-19T09:00:00+00:00')",
            [],
        )
        .unwrap();
        assert!(
            db.execute(
                "INSERT INTO work_evidence(run_id,evidence_ref,tool_call_id,connector_instance_id,
                     canonical_resource_id,source_kind,tool_definition_digest,result_digest,succeeded,observed_at)
                 VALUES('run-1','ev-1','call-2','github:acme','acme/bridge#9','github_issue',
                        'sha256:tool','sha256:other',1,'2026-08-19T09:00:00+00:00')",
                [],
            )
            .is_err(),
            "an evidence reference must mean one thing inside a run"
        );

        let insert_task = |id: &str, fingerprint: Option<&str>| {
            db.execute(
                "INSERT INTO work_tasks(id,fingerprint,connector_instance_id,canonical_resource_id,
                     source_kind,title,why,rank,confidence_bps,created_at,updated_at)
                 VALUES(?1,?2,'github:acme','acme/bridge#204','github_issue','Review','because',
                        1,8200,'now','now')",
                params![id, fingerprint],
            )
        };
        insert_task("t-1", Some("fp-1")).unwrap();
        assert!(
            insert_task("t-2", Some("fp-1")).is_err(),
            "two tasks cannot share a fingerprint"
        );
        insert_task("t-3", None).unwrap();
        insert_task("t-4", None).expect(
            "ephemeral tasks have no fingerprint, and SQLite counts NULLs as distinct",
        );

        db.execute(
            "INSERT INTO work_fact_cache(kind,cache_key,status,observed_at)
             VALUES('workspace_behind_base','w','ok','2026-08-19T09:00:00+00:00')",
            [],
        )
        .unwrap();
        assert!(
            db.execute(
                "INSERT INTO work_fact_cache(kind,cache_key,status,observed_at)
                 VALUES('workspace_behind_base','w','failed','2026-08-19T09:05:00+00:00')",
                [],
            )
            .is_err(),
            "one observation per kind and key; a second row would make freshness ambiguous"
        );
    }

    #[test]
    fn work_tables_cascade_from_their_run() {
        let db = open(Path::new(":memory:")).unwrap();
        insert_work_run(&db, "run-1");
        db.execute(
            "INSERT INTO work_brief_sources(run_id,connector_instance_id,connector_family,status)
             VALUES('run-1','github:acme','github','succeeded')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO work_evidence(run_id,evidence_ref,tool_call_id,connector_instance_id,
                 canonical_resource_id,source_kind,tool_definition_digest,result_digest,succeeded,observed_at)
             VALUES('run-1','ev-1','call-1','github:acme','acme/bridge#204','github_issue',
                    'sha256:tool','sha256:result',1,'2026-08-19T09:00:00+00:00')",
            [],
        )
        .unwrap();
        db.execute("DELETE FROM work_brief_runs WHERE id='run-1'", []).unwrap();
        for table in ["work_brief_sources", "work_evidence"] {
            let remaining: i64 = db
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))
                .unwrap();
            assert_eq!(remaining, 0, "{table} must not outlive its run");
        }
    }

    #[test]
    fn migration_24_rolls_back_every_object_when_one_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        let mut db = Connection::open(&path).unwrap();
        // A view cannot be indexed, and `CREATE TABLE IF NOT EXISTS` over a view
        // is a silent no-op — so this fails partway through the batch, after
        // `work_brief_runs` has already been created.
        db.execute_batch("CREATE VIEW work_brief_sources AS SELECT 1 AS run_id;")
            .unwrap();
        let transaction = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert!(migration_24_work_board(&transaction).is_err());
        drop(transaction);
        let created: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'work_%' AND type='table'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(created, 0, "a half-applied Work schema must not survive the failure");
    }

    #[test]
    fn migration_failure_rolls_back_objects_and_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        let mut db = Connection::open(&path).unwrap();
        let transaction = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        migration_1_current_schema(&transaction).unwrap();
        transaction
            .execute(
                "INSERT INTO schema_version(version,applied_at) VALUES(1,'now')",
                [],
            )
            .unwrap();
        transaction.commit().unwrap();
        db.execute("CREATE TABLE session_entries(blocker TEXT)", [])
            .unwrap();
        drop(db);
        assert!(open(&path).is_err());
        let db = Connection::open(&path).unwrap();
        assert_eq!(migration_versions(&db), vec![1]);
        let partial: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='session_heads')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!partial);
    }

    #[test]
    fn backfills_agent_events_as_linear_session_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        create_legacy_fixture(&path);
        let db = open(&path).unwrap();
        let entries = session_entries(&db, "s").unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].sequence, 1);
        assert_eq!(entries[0].parent_entry_id, None);
        assert_eq!(entries[1].parent_entry_id.as_deref(), Some(&*entries[0].id));
        assert_eq!(entries[1].payload["providerMeta"]["rawId"], 2);
        assert_eq!(entries[1].provider_event_id.as_deref(), Some("m2"));
        assert!(entries
            .iter()
            .all(|entry| entry.semantic_schema_version == 1));
        let head = session_head(&db, "s").unwrap().unwrap();
        assert_eq!(head.active_entry_id.as_deref(), Some(&*entries[1].id));
        assert_eq!(head.native_provider_session_id.as_deref(), Some("native-s"));
        assert_eq!(head.restoration_mode, RestorationMode::Fresh);
        assert_eq!(head.resume_eligibility, ResumeEligibility::Native);
    }

    #[test]
    fn session_entry_sequence_is_unique_and_append_assigns_next_sequence() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("bridge.db")).unwrap();
        seed_workspace(&db);
        let first = append_session_entry(
            &db,
            "s",
            None,
            "user.message",
            &json!({"text":"one"}),
            None,
            "eligible",
            Some(1),
        )
        .unwrap();
        let second = append_session_entry(
            &db,
            "s",
            Some(&first.id),
            "assistant.message",
            &json!({"text":"two"}),
            None,
            "eligible",
            Some(1),
        )
        .unwrap();
        assert_eq!((first.sequence, second.sequence), (1, 2));
        assert_eq!(first.semantic_schema_version, SEMANTIC_EVENT_SCHEMA_VERSION);
        assert_eq!(
            second.semantic_schema_version,
            SEMANTIC_EVENT_SCHEMA_VERSION
        );
        assert_eq!(
            session_head(&db, "s").unwrap().unwrap().active_entry_id,
            Some(second.id.clone())
        );
        let duplicate = db.execute(
            "INSERT INTO session_entries(id,session_id,sequence,kind,created_at) VALUES('duplicate','s',2,'user.message','now')",
            [],
        );
        assert!(duplicate.is_err());
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("bridge.db")).unwrap();
        assert_eq!(
            db.query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(append_session_entry(
            &db,
            "missing",
            None,
            "user.message",
            &json!({}),
            None,
            "eligible",
            None,
        )
        .is_err());
    }

    #[test]
    fn queries_knowledge_leases_and_usage_ledger() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("bridge.db")).unwrap();
        seed_workspace(&db);
        let entry = append_session_entry(
            &db,
            "s",
            None,
            "user.message",
            &json!({"text":"remember"}),
            None,
            "eligible",
            None,
        )
        .unwrap();
        let knowledge = TaskKnowledge {
            id: "k".into(),
            workspace_id: "w".into(),
            session_id: Some("s".into()),
            kind: "decision".into(),
            body: "Use SQLite".into(),
            source_entry_id: Some(entry.id),
            superseded_by: None,
            created_at: "now".into(),
        };
        insert_task_knowledge(&db, &knowledge).unwrap();
        assert_eq!(task_knowledge(&db, "w").unwrap(), vec![knowledge]);

        let lease = WorkerLease {
            session_id: "s".into(),
            workspace_id: "w".into(),
            role: "implementation".into(),
            capability_tier: "standard".into(),
            task_family: "implementation".into(),
            owned_paths: json!(["src/**"]),
            write_mode: "isolated".into(),
            lease_status: "active".into(),
            expires_at: None,
            created_at: "now".into(),
            updated_at: "now".into(),
        };
        upsert_worker_lease(&db, &lease).unwrap();
        assert_eq!(worker_leases(&db, "w").unwrap(), vec![lease]);

        let compilation = PromptCompilationRecord {
            id: 0,
            session_id: "s".into(),
            turn_id: None,
            prefix_id: "prefix-1".into(),
            prefix_hash: "hash-1".into(),
            schema_version: 1,
            prefix_bytes: 400,
            prefix_token_estimate: 100,
            harness: "codex".into(),
            model: Some("gpt".into()),
            role: "worker:implementation".into(),
            task_family: "implementation".into(),
            restoration_mode: "fresh".into(),
            cross_harness_reuse: "not_applicable".into(),
            created_at: "now".into(),
        };
        let compilation_id = record_prompt_compilation(&db, &compilation).unwrap();
        let stored_compilation = latest_prompt_compilation(&db, "s").unwrap().unwrap();
        assert_eq!(stored_compilation.id, compilation_id);
        assert_eq!(stored_compilation.prefix_hash, "hash-1");
        assert_eq!(stored_compilation.prefix_token_estimate, 100);
        assert!(bind_latest_prompt_compilation_to_turn(&db, "s", "turn-1").unwrap());
        assert_eq!(
            prompt_compilation_for_turn(&db, "s", "turn-1")
                .unwrap()
                .unwrap()
                .id,
            compilation_id
        );
        let mut replacement = compilation.clone();
        replacement.prefix_id = "prefix-unsent".into();
        replacement.prefix_hash = "hash-unsent".into();
        let replacement_id = record_prompt_compilation(&db, &replacement).unwrap();
        assert!(delete_prompt_compilation(&db, replacement_id).unwrap());
        assert!(!delete_prompt_compilation(&db, replacement_id).unwrap());
        assert_eq!(
            latest_prompt_compilation(&db, "s").unwrap().unwrap().id,
            compilation_id
        );

        let usage = UsageLedgerRow {
            id: 0,
            workspace_id: "w".into(),
            session_id: Some("s".into()),
            turn_id: Some("turn-1".into()),
            input_tokens: Some(10),
            output_tokens: Some(5),
            cache_read_tokens: Some(2),
            cache_write_tokens: None,
            uncached_input_tokens: Some(8),
            context_percent: Some(25),
            capability_units: 3,
            runtime_ms: Some(100),
            cost_microusd: Some(12_345),
            cost_source: Some("provider_reported".into()),
            stable_prefix_id: Some("prefix-1".into()),
            stable_prefix_hash: Some("hash-1".into()),
            prompt_schema_version: Some(1),
            prefix_token_estimate: Some(100),
            harness: Some("codex".into()),
            model: Some("gpt".into()),
            role: Some("worker:implementation".into()),
            task_family: Some("implementation".into()),
            restoration_mode: Some("fresh".into()),
            cross_harness_reuse: Some("not_applicable".into()),
            source: "codex".into(),
            created_at: "now".into(),
        };
        let id = append_usage_ledger(&db, &usage).unwrap();
        let rows = usage_ledger(&db, "w", Some("s")).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].turn_id.as_deref(), Some("turn-1"));
        assert_eq!(rows[0].capability_units, 3);
        assert_eq!(rows[0].cost_microusd, Some(12_345));
        assert_eq!(rows[0].uncached_input_tokens, Some(8));
        assert_eq!(rows[0].stable_prefix_hash.as_deref(), Some("hash-1"));
    }

    #[test]
    fn durable_worker_bookkeeping_deduplicates_counts_and_queues_fifo() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("bridge.db")).unwrap();
        seed_workspace(&db);
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id,depth) VALUES('child','w','claude','Worker','working','reported','s',1)", []).unwrap();

        assert!(claim_delegation_receipt(&db, "s", "message-1").unwrap());
        assert!(!claim_delegation_receipt(&db, "s", "message-1").unwrap());
        let runtime = WorkerRuntimeRecord {
            session_id: "child".into(),
            parent_session_id: "s".into(),
            lifecycle_state: "working".into(),
            task_family: "implementation".into(),
            compatibility_key: "w|implementation|claude|standard|implementation|src/**".into(),
            result_status: "pending".into(),
            retry_count: 0,
            warm_until: None,
            worktree_path: None,
            worktree_branch: None,
            last_result: None,
            last_activity_at: Some("active-now".into()),
            updated_at: "now".into(),
        };
        upsert_worker_runtime(&db, &runtime).unwrap();
        assert_eq!(worker_runtime(&db, "child").unwrap(), Some(runtime));
        assert_eq!(outstanding_children(&db, "s").unwrap(), 1);

        for id in ["q1", "q2"] {
            enqueue_worker_request(
                &db,
                &QueuedWorkerRequest {
                    id: id.into(),
                    parent_session_id: "s".into(),
                    workspace_id: "w".into(),
                    turn_id: "turn".into(),
                    request: json!({"role":"implementation"}),
                    actual_model: "runtime-model".into(),
                    queue_status: "queued".into(),
                    sequence: 0,
                    dispatched_session_id: None,
                    attempt_count: 0,
                    expires_at: "2099-01-01T00:00:00+00:00".into(),
                    blocked_at: None,
                    claimed_at: None,
                    last_error: None,
                    created_at: "now".into(),
                    updated_at: "now".into(),
                },
            )
            .unwrap();
        }
        let queued = queued_worker_requests(&db, "w").unwrap();
        assert_eq!(
            queued
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["q1", "q2"]
        );
        assert!(queued[0].sequence < queued[1].sequence);
    }

    #[test]
    fn busy_wal_aborts_before_migration_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        create_legacy_fixture(&path);

        let reader = Connection::open(&path).unwrap();
        reader
            .execute_batch("PRAGMA journal_mode=WAL; BEGIN")
            .unwrap();
        let _: i64 = reader
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .unwrap();
        let writer = Connection::open(&path).unwrap();
        writer
            .execute("UPDATE sessions SET label='Changed' WHERE id='s'", [])
            .unwrap();
        drop(writer);

        let error = open(&path).unwrap_err().to_string();
        assert!(error.contains("WAL is busy"), "unexpected error: {error}");
        assert!(backup_paths(dir.path()).is_empty());
        let raw = Connection::open(&path).unwrap();
        let has_versions: bool = raw
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_version')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!has_versions);
        drop(raw);
        reader.execute_batch("ROLLBACK").unwrap();
    }
}

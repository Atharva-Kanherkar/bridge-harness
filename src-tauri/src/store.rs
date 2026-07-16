use crate::{model::*, BridgeError};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Command,
};
use uuid::Uuid;

const LATEST_SCHEMA_VERSION: i64 = 14;

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
    connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
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
    let mut statement = transaction.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    if !columns.iter().any(|existing| existing == column) {
        transaction.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {definition}"
        ))?;
    }
    Ok(())
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
    add_column_if_missing(transaction, "worker_queue", "attempt_count", "INTEGER NOT NULL DEFAULT 0")?;
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
pub fn harness(value: &str) -> Harness {
    match value {
        "claude" => Harness::Claude,
        "codex" => Harness::Codex,
        _ => Harness::Shell,
    }
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
pub fn harness_name(value: &Harness) -> &'static str {
    match value {
        Harness::Claude => "claude",
        Harness::Codex => "codex",
        Harness::Shell => "shell",
    }
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
    let path: Option<String> = db.query_row(
        "SELECT COALESCE(s.cwd,w.path) FROM sessions s LEFT JOIN workspaces w ON w.id=s.workspace_id WHERE s.id=?1",
        params![session_id],
        |row| row.get(0),
    ).optional()?.flatten();
    let Some(path) = path else {
        return Ok(serde_json::json!({"status":"unavailable"}));
    };
    Ok(repository_state_for_path(Path::new(&path)))
}

pub fn repository_state_for_path(path: &Path) -> serde_json::Value {
    let head = Command::new("git").args(["rev-parse", "HEAD"]).current_dir(path).output();
    let status = Command::new("git")
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .current_dir(path)
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
        "INSERT INTO worker_runtime(session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,result_status,retry_count,warm_until,worktree_path,worktree_branch,last_result,updated_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
         ON CONFLICT(session_id) DO UPDATE SET parent_session_id=excluded.parent_session_id,lifecycle_state=excluded.lifecycle_state,task_family=excluded.task_family,compatibility_key=excluded.compatibility_key,result_status=excluded.result_status,retry_count=excluded.retry_count,warm_until=excluded.warm_until,worktree_path=excluded.worktree_path,worktree_branch=excluded.worktree_branch,last_result=excluded.last_result,updated_at=excluded.updated_at",
        params![runtime.session_id,runtime.parent_session_id,runtime.lifecycle_state,runtime.task_family,runtime.compatibility_key,runtime.result_status,runtime.retry_count,runtime.warm_until,runtime.worktree_path,runtime.worktree_branch,runtime.last_result.as_ref().map(serde_json::Value::to_string),runtime.updated_at],
    )?;
    Ok(())
}

pub fn worker_runtime(
    db: &Connection,
    session_id: &str,
) -> Result<Option<WorkerRuntimeRecord>, BridgeError> {
    db.query_row(
        "SELECT session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,result_status,retry_count,warm_until,worktree_path,worktree_branch,last_result,updated_at FROM worker_runtime WHERE session_id=?1",
        params![session_id],
        |row| Ok(WorkerRuntimeRecord { session_id:row.get(0)?, parent_session_id:row.get(1)?, lifecycle_state:row.get(2)?, task_family:row.get(3)?, compatibility_key:row.get(4)?, result_status:row.get(5)?, retry_count:row.get(6)?, warm_until:row.get(7)?, worktree_path:row.get(8)?, worktree_branch:row.get(9)?, last_result:row.get::<_,Option<String>>(10)?.and_then(|value| serde_json::from_str(&value).ok()), updated_at:row.get(11)? }),
    ).optional().map_err(BridgeError::from)
}

pub fn worker_runtimes(
    db: &Connection,
    workspace_id: &str,
) -> Result<Vec<WorkerRuntimeRecord>, BridgeError> {
    query_with_params(
        db,
        "SELECT r.session_id,r.parent_session_id,r.lifecycle_state,r.task_family,r.compatibility_key,r.result_status,r.retry_count,r.warm_until,r.worktree_path,r.worktree_branch,r.last_result,r.updated_at
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
                updated_at: row.get(11)?,
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

pub fn enqueue_outbox(transaction: &Transaction<'_>, message: &OutboxMessage) -> Result<(), BridgeError> {
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
        "INSERT INTO usage_ledger(workspace_id,session_id,turn_id,input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,context_percent,capability_units,runtime_ms,source,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        params![
            usage.workspace_id,
            usage.session_id,
            usage.turn_id,
            usage.input_tokens,
            usage.output_tokens,
            usage.cache_read_tokens,
            usage.cache_write_tokens,
            usage.context_percent,
            usage.capability_units,
            usage.runtime_ms,
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
    let sql = "SELECT id,workspace_id,session_id,turn_id,input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,context_percent,capability_units,runtime_ms,source,created_at
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
            context_percent: row.get(8)?,
            capability_units: row.get(9)?,
            runtime_ms: row.get(10)?,
            source: row.get(11)?,
            created_at: row.get(12)?,
        })
    })
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
    let parent_entry_id: Option<String> = transaction
        .query_row(
            "SELECT active_entry_id FROM session_heads WHERE session_id=?1",
            params![session_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let trace_id: String = transaction.query_row("SELECT COALESCE(trace_id,id) FROM sessions WHERE id=?1", params![session_id], |row| row.get(0)).unwrap_or_else(|_| session_id.to_owned());
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
    } else if final_kind == "tool.started" || final_kind == "tool.completed" || final_kind == "approval.requested" || final_kind == "approval.resolved" || final_kind == "delegation.requested" || final_kind == "delegation.approved" || final_kind == "delegation.rejected" || final_kind == "worker.result" {
        // Keep as is, it maps directly.
    } else if final_kind.ends_with(".delta") || final_kind.ends_with(".progress") || final_kind == "turn.started" || final_kind == "turn.completed" || final_kind == "usage.updated" || final_kind == "plan.updated" {
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
        &transaction,
        session_id,
        parent_entry_id.as_deref(),
        final_kind,
        &payload,
        event.item_id.as_deref(),
        "eligible",
        None,
    )?;
    transaction.commit()?;
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
                    "SELECT type,name,tbl_name FROM sqlite_master
                     WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name",
                )
                .unwrap();
            let rows = statement
                .query_map([], |row| {
                    Ok(format!(
                        "{}:{}:{}",
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?
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
        let committed = session_event(&primary, "s", &event, &json!({"adapter":"codex"}))
            .unwrap();
        let write_lock = primary.unchecked_transaction().unwrap();
        write_lock
            .execute("UPDATE sessions SET label='locked' WHERE id='s'", [])
            .unwrap();
        let span = telemetry_span("trace", "s", "codex", &event, &committed.created_at);
        assert_eq!(append_telemetry_batch(&telemetry, &[span.clone()]).unwrap(), 1);
        write_lock.rollback().unwrap();
        assert_eq!(telemetry.query_row("SELECT COUNT(*) FROM telemetry_spans", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(primary.query_row("SELECT COUNT(*) FROM session_entries", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(primary.query_row("SELECT COUNT(*) FROM telemetry_spans", [], |row| row.get::<_, i64>(0)).unwrap(), 0);

        telemetry.execute("DROP TABLE telemetry_spans", []).unwrap();
        assert!(append_telemetry_batch(&telemetry, &[span]).is_err());
        assert_eq!(primary.query_row("SELECT COUNT(*) FROM session_entries", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn history_snapshot_is_consistent_and_checksum_detects_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let primary_path = dir.path().join("bridge.db");
        let primary = open(&primary_path).unwrap();
        event(&primary, "test", "history.saved", "entity", "durable").unwrap();
        let readonly = Connection::open_with_flags(
            &primary_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .unwrap();
        let (snapshot, manifest) =
            export_history_snapshot(&readonly, &dir.path().join("snapshots")).unwrap();
        assert!(verify_history_snapshot(&snapshot, &manifest).unwrap());
        let snapshot_db = Connection::open(&snapshot).unwrap();
        assert_eq!(snapshot_db.query_row("SELECT body FROM events WHERE kind='history.saved'", [], |row| row.get::<_, String>(0)).unwrap(), "durable");
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
        assert_eq!(migration_versions(&db), vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]);
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
        assert_eq!(migration_versions(&db), vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]);
        assert_eq!(backup_paths(dir.path()).len(), 1);
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
        for (id, parent) in [("root", None), ("boundary", Some("root")), ("mid", Some("root")), ("resumed", Some("root"))] {
            db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id,continuation_fidelity) VALUES(?1,'w','codex','Session','stopped','reported',?2,'native')", params![id,parent]).unwrap();
        }
        for (id, mode) in [("root","fresh"),("boundary","checkpoint_restored"),("mid","fresh"),("resumed","native")] {
            db.execute("INSERT INTO session_heads(session_id,restoration_mode,updated_at) VALUES(?1,?2,'now')", params![id,mode]).unwrap();
        }
        let transaction = db.transaction().unwrap();
        migration_10_continuation_fidelity(&transaction).unwrap();
        transaction.commit().unwrap();
        let values = query(&db, "SELECT continuation_fidelity FROM sessions ORDER BY CASE id WHEN 'root' THEN 1 WHEN 'boundary' THEN 2 WHEN 'mid' THEN 3 ELSE 4 END", |row| row.get::<_,String>(0)).unwrap();
        assert_eq!(values, vec!["native", "projected_at_boundary", "projected_mid_turn", "native"]);
    }

    #[test]
    fn human_blocked_queue_migration_preserves_existing_rows() {
        let mut db = Connection::open(":memory:").unwrap();
        db.execute_batch("CREATE TABLE worker_queue(id TEXT PRIMARY KEY,queue_status TEXT NOT NULL,expires_at TEXT); INSERT INTO worker_queue VALUES('q','queued','2099-01-01T00:00:00+00:00');").unwrap();
        let transaction = db.transaction().unwrap();
        migration_11_human_blocked_queue(&transaction).unwrap();
        transaction.commit().unwrap();
        let row = db.query_row("SELECT queue_status,expires_at,blocked_at FROM worker_queue WHERE id='q'", [], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,Option<String>>(2)?))).unwrap();
        assert_eq!(row, ("queued".into(), "2099-01-01T00:00:00+00:00".into(), None));
    }

    #[test]
    fn adapter_process_claim_migration_preserves_existing_sessions() {
        let mut db = Connection::open(":memory:").unwrap();
        db.execute_batch("CREATE TABLE sessions(id TEXT PRIMARY KEY,status TEXT NOT NULL); INSERT INTO sessions VALUES('s','working');").unwrap();
        let transaction = db.transaction().unwrap();
        migration_12_adapter_process_claims(&transaction).unwrap();
        transaction.commit().unwrap();
        let row = db.query_row("SELECT status,adapter_pid,adapter_process_identity FROM sessions WHERE id='s'", [], |row| Ok((row.get::<_,String>(0)?,row.get::<_,Option<i64>>(1)?,row.get::<_,Option<String>>(2)?))).unwrap();
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
        assert!(entries.iter().all(|entry| entry.semantic_schema_version == 1));
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
        assert_eq!(second.semantic_schema_version, SEMANTIC_EVENT_SCHEMA_VERSION);
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

        let usage = UsageLedgerRow {
            id: 0,
            workspace_id: "w".into(),
            session_id: Some("s".into()),
            turn_id: Some("turn-1".into()),
            input_tokens: Some(10),
            output_tokens: Some(5),
            cache_read_tokens: Some(2),
            cache_write_tokens: None,
            context_percent: Some(25),
            capability_units: 3,
            runtime_ms: Some(100),
            source: "codex".into(),
            created_at: "now".into(),
        };
        let id = append_usage_ledger(&db, &usage).unwrap();
        let rows = usage_ledger(&db, "w", Some("s")).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].turn_id.as_deref(), Some("turn-1"));
        assert_eq!(rows[0].capability_units, 3);
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
            updated_at: "now".into(),
        };
        upsert_worker_runtime(&db, &runtime).unwrap();
        assert_eq!(worker_runtime(&db, "child").unwrap(), Some(runtime));
        assert_eq!(outstanding_children(&db, "s").unwrap(), 1);

        for id in ["q1", "q2"] {
            enqueue_worker_request(&db, &QueuedWorkerRequest {
                id: id.into(), parent_session_id: "s".into(), workspace_id: "w".into(), turn_id: "turn".into(), request: json!({"role":"implementation"}), actual_model: "runtime-model".into(), queue_status: "queued".into(), sequence: 0, dispatched_session_id: None, attempt_count: 0, expires_at: "2099-01-01T00:00:00+00:00".into(), blocked_at: None, claimed_at: None, last_error: None, created_at: "now".into(), updated_at: "now".into(),
            }).unwrap();
        }
        let queued = queued_worker_requests(&db, "w").unwrap();
        assert_eq!(queued.iter().map(|item| item.id.as_str()).collect::<Vec<_>>(), vec!["q1", "q2"]);
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

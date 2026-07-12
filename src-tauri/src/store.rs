use crate::{model::*, BridgeError};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
use uuid::Uuid;

const LATEST_SCHEMA_VERSION: i64 = 2;

pub fn open(path: &Path) -> Result<Connection, BridgeError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut connection = Connection::open(path)?;
    connection.execute_batch("PRAGMA foreign_keys=ON;")?;
    run_migrations(&mut connection, path)?;
    connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    connection.execute(
        "UPDATE sessions SET status='stopped', ended_at=?1 WHERE status IN ('working','waiting')",
        params![Utc::now().to_rfc3339()],
    )?;
    connection.execute(
        "UPDATE workspaces SET status='stopped' WHERE status IN ('working','waiting')",
        [],
    )?;
    Ok(connection)
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
    let sessions = query(db, "SELECT id,workspace_id,harness,label,status,started_at,ended_at,context_percent,usage_percent,metric_source,provider_session_id,active_turn_id,model,effort,parent_session_id,depth FROM sessions ORDER BY rowid", |r| Ok(Session { id:r.get(0)?, workspace_id:r.get(1)?, harness:harness(&r.get::<_,String>(2)?), label:r.get(3)?, status:status(&r.get::<_,String>(4)?), started_at:r.get(5)?, ended_at:r.get(6)?, context_percent:r.get(7)?, usage_percent:r.get(8)?, metric_source:r.get(9)?, provider_session_id:r.get(10)?, active_turn_id:r.get(11)?, model:r.get(12)?, effort:r.get(13)?, parent_session_id:r.get(14)?, depth:r.get(15)? }))?;
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
    let agent_events = query(db, "SELECT id,session_id,sequence,protocol_version,kind,item_id,role,status,title,text,data,provider_meta,created_at FROM agent_events ORDER BY session_id,sequence", |r| Ok(AgentEvent { id:r.get(0)?, session_id:r.get(1)?, sequence:r.get(2)?, protocol_version:r.get(3)?, kind:r.get(4)?, item_id:r.get(5)?, role:r.get(6)?, status:r.get(7)?, title:r.get(8)?, text:r.get(9)?, data:serde_json::from_str(&r.get::<_,String>(10)?).unwrap_or(serde_json::Value::Null), provider_meta:serde_json::from_str(&r.get::<_,String>(11)?).unwrap_or(serde_json::Value::Null), created_at:r.get(12)? }))?;
    Ok(BridgeState {
        projects,
        workspaces,
        sessions,
        events,
        agent_events,
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
        "working" => SessionStatus::Working,
        "waiting" => SessionStatus::Waiting,
        "ready" => SessionStatus::Ready,
        "failed" => SessionStatus::Failed,
        "stopped" => SessionStatus::Stopped,
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
    let entry = SessionEntry {
        id: Uuid::new_v4().to_string(),
        session_id: session_id.to_owned(),
        parent_entry_id: parent_entry_id.map(str::to_owned),
        sequence,
        kind: kind.to_owned(),
        payload: payload.clone(),
        provider_event_id: provider_event_id.map(str::to_owned),
        context_visibility: context_visibility.to_owned(),
        token_estimate,
        created_at: Utc::now().to_rfc3339(),
    };
    transaction.execute(
        "INSERT INTO session_entries(id,session_id,parent_entry_id,sequence,kind,payload,provider_event_id,context_visibility,token_estimate,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            entry.id,
            entry.session_id,
            entry.parent_entry_id,
            entry.sequence,
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

pub fn session_entries(
    db: &Connection,
    session_id: &str,
) -> Result<Vec<SessionEntry>, BridgeError> {
    query_with_params(
        db,
        "SELECT id,session_id,parent_entry_id,sequence,kind,payload,provider_event_id,context_visibility,token_estimate,created_at
         FROM session_entries WHERE session_id=?1 ORDER BY sequence",
        params![session_id],
        |row| {
            Ok(SessionEntry {
                id: row.get(0)?,
                session_id: row.get(1)?,
                parent_entry_id: row.get(2)?,
                sequence: row.get(3)?,
                kind: row.get(4)?,
                payload: parse_json_column(row, 5),
                provider_event_id: row.get(6)?,
                context_visibility: row.get(7)?,
                token_estimate: row.get(8)?,
                created_at: row.get(9)?,
            })
        },
    )
}

pub fn session_head(db: &Connection, session_id: &str) -> Result<Option<SessionHead>, BridgeError> {
    db.query_row(
        "SELECT session_id,active_entry_id,native_provider_session_id,restoration_mode,latest_checkpoint_entry_id,updated_at
         FROM session_heads WHERE session_id=?1",
        params![session_id],
        |row| {
            Ok(SessionHead {
                session_id: row.get(0)?,
                active_entry_id: row.get(1)?,
                native_provider_session_id: row.get(2)?,
                restoration_mode: row.get(3)?,
                latest_checkpoint_entry_id: row.get(4)?,
                updated_at: row.get(5)?,
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
        "INSERT INTO worker_leases(session_id,workspace_id,role,capability_tier,owned_paths,write_mode,lease_status,expires_at,created_at,updated_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
         ON CONFLICT(session_id) DO UPDATE SET workspace_id=excluded.workspace_id,role=excluded.role,capability_tier=excluded.capability_tier,owned_paths=excluded.owned_paths,write_mode=excluded.write_mode,lease_status=excluded.lease_status,expires_at=excluded.expires_at,updated_at=excluded.updated_at",
        params![
            lease.session_id,
            lease.workspace_id,
            lease.role,
            lease.capability_tier,
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
        "SELECT session_id,workspace_id,role,capability_tier,owned_paths,write_mode,lease_status,expires_at,created_at,updated_at
         FROM worker_leases WHERE workspace_id=?1 ORDER BY created_at,session_id",
        params![workspace_id],
        |row| {
            Ok(WorkerLease {
                session_id: row.get(0)?,
                workspace_id: row.get(1)?,
                role: row.get(2)?,
                capability_tier: row.get(3)?,
                owned_paths: parse_json_column(row, 4),
                write_mode: row.get(5)?,
                lease_status: row.get(6)?,
                expires_at: row.get(7)?,
                created_at: row.get(8)?,
                updated_at: row.get(9)?,
            })
        },
    )
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

pub fn agent_event(
    db: &Connection,
    session_id: &str,
    event: &crate::agent::NormalizedEvent,
    provider_meta: &serde_json::Value,
) -> Result<AgentEvent, BridgeError> {
    event.validate().map_err(BridgeError::Invalid)?;
    let transaction = db.unchecked_transaction()?;
    let sequence: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(sequence),0)+1 FROM agent_events WHERE session_id=?1",
        params![session_id],
        |r| r.get(0),
    )?;
    let created_at = Utc::now().to_rfc3339();
    transaction.execute(
        "INSERT INTO agent_events(session_id,sequence,protocol_version,kind,item_id,role,status,title,text,data,provider_meta,created_at) VALUES(?1,?2,1,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![session_id,sequence,event.kind,event.item_id,event.role,event.status,event.title,event.text,event.data.to_string(),provider_meta.to_string(),created_at],
    )?;
    let agent_event = AgentEvent {
        id: transaction.last_insert_rowid(),
        session_id: session_id.into(),
        sequence,
        protocol_version: 1,
        kind: event.kind.clone(),
        item_id: event.item_id.clone(),
        role: event.role.clone(),
        status: event.status.clone(),
        title: event.title.clone(),
        text: event.text.clone(),
        data: event.data.clone(),
        provider_meta: provider_meta.clone(),
        created_at: created_at.clone(),
    };
    let parent_entry_id: Option<String> = transaction
        .query_row(
            "SELECT active_entry_id FROM session_heads WHERE session_id=?1",
            params![session_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let payload = serde_json::json!({
        "protocolVersion": 1,
        "itemId": event.item_id,
        "role": event.role,
        "status": event.status,
        "title": event.title,
        "text": event.text,
        "data": event.data,
        "providerMeta": provider_meta,
    });
    append_session_entry_tx(
        &transaction,
        session_id,
        parent_entry_id.as_deref(),
        &event.kind,
        &payload,
        event.item_id.as_deref(),
        "eligible",
        None,
    )?;
    transaction.commit()?;
    Ok(agent_event)
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
    fn migrates_current_schema_fixture_idempotently_and_creates_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        create_legacy_fixture(&path);
        let db = open(&path).unwrap();
        assert_eq!(migration_versions(&db), vec![1, 2]);
        assert_eq!(state(&db).unwrap().agent_events.len(), 2);
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
        assert_eq!(migration_versions(&db), vec![1, 2]);
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
        let head = session_head(&db, "s").unwrap().unwrap();
        assert_eq!(head.active_entry_id.as_deref(), Some(&*entries[1].id));
        assert_eq!(head.native_provider_session_id.as_deref(), Some("native-s"));
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
    fn agent_event_dual_write_is_atomic_and_equivalent() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("bridge.db")).unwrap();
        seed_workspace(&db);
        let first = crate::agent::normalize_codex_message(
            &json!({"method":"item/agentMessage/delta","params":{"itemId":"m","delta":"hel"}}),
        );
        agent_event(&db, "s", &first[0], &json!({"provider":"codex"})).unwrap();
        let snapshot = state(&db).unwrap();
        let entries = session_entries(&db, "s").unwrap();
        assert_eq!(snapshot.agent_events.len(), 1);
        assert_eq!(entries.len(), 1);
        assert_eq!(snapshot.agent_events[0].sequence, 1);
        assert_eq!(entries[0].sequence, 1);
        assert_eq!(entries[0].kind, snapshot.agent_events[0].kind);
        assert_eq!(entries[0].payload["data"], snapshot.agent_events[0].data);
        assert_eq!(entries[0].payload["providerMeta"]["provider"], "codex");

        db.execute_batch(
            "CREATE TRIGGER reject_forest_insert BEFORE INSERT ON session_entries
             BEGIN SELECT RAISE(FAIL, 'injected forest failure'); END;",
        )
        .unwrap();
        assert!(agent_event(&db, "s", &first[0], &json!({"provider":"codex"})).is_err());
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM agent_events", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM session_entries", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
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

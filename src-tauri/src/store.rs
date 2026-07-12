use crate::{model::*, BridgeError};
use chrono::Utc;
use rusqlite::{params, Connection};
use std::path::Path;

pub fn open(path: &Path) -> Result<Connection, BridgeError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let connection = Connection::open(path)?;
    connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
        CREATE TABLE IF NOT EXISTS projects (id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE, created_at TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS workspaces (id TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id), city TEXT NOT NULL, title TEXT NOT NULL, branch TEXT NOT NULL, path TEXT NOT NULL UNIQUE, status TEXT NOT NULL, dirty_files INTEGER NOT NULL DEFAULT 0, additions INTEGER NOT NULL DEFAULT 0, deletions INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS sessions (id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL REFERENCES workspaces(id), harness TEXT NOT NULL, label TEXT NOT NULL, status TEXT NOT NULL, started_at TEXT, ended_at TEXT, context_percent INTEGER, usage_percent INTEGER, metric_source TEXT NOT NULL DEFAULT 'estimated');
        CREATE TABLE IF NOT EXISTS events (id INTEGER PRIMARY KEY AUTOINCREMENT, source TEXT NOT NULL, kind TEXT NOT NULL, entity_id TEXT NOT NULL, body TEXT NOT NULL, created_at TEXT NOT NULL);")?;
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS agent_events (
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
    ); CREATE INDEX IF NOT EXISTS idx_agent_events_session ON agent_events(session_id, sequence);",
    )?;
    let _ = connection.execute(
        "ALTER TABLE sessions ADD COLUMN provider_session_id TEXT",
        [],
    );
    let _ = connection.execute("ALTER TABLE sessions ADD COLUMN active_turn_id TEXT", []);
    let _ = connection.execute("ALTER TABLE sessions ADD COLUMN model TEXT", []);
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
    let sessions = query(db, "SELECT id,workspace_id,harness,label,status,started_at,ended_at,context_percent,usage_percent,metric_source,provider_session_id,active_turn_id,model FROM sessions ORDER BY rowid", |r| Ok(Session { id:r.get(0)?, workspace_id:r.get(1)?, harness:harness(&r.get::<_,String>(2)?), label:r.get(3)?, status:status(&r.get::<_,String>(4)?), started_at:r.get(5)?, ended_at:r.get(6)?, context_percent:r.get(7)?, usage_percent:r.get(8)?, metric_source:r.get(9)?, provider_session_id:r.get(10)?, active_turn_id:r.get(11)?, model:r.get(12)? }))?;
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

pub fn agent_event(
    db: &Connection,
    session_id: &str,
    event: &crate::agent::NormalizedEvent,
    provider_meta: &serde_json::Value,
) -> Result<AgentEvent, BridgeError> {
    event.validate().map_err(BridgeError::Invalid)?;
    let sequence: i64 = db.query_row(
        "SELECT COALESCE(MAX(sequence),0)+1 FROM agent_events WHERE session_id=?1",
        params![session_id],
        |r| r.get(0),
    )?;
    let created_at = Utc::now().to_rfc3339();
    db.execute(
        "INSERT INTO agent_events(session_id,sequence,protocol_version,kind,item_id,role,status,title,text,data,provider_meta,created_at) VALUES(?1,?2,1,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![session_id,sequence,event.kind,event.item_id,event.role,event.status,event.title,event.text,event.data.to_string(),provider_meta.to_string(),created_at],
    )?;
    Ok(AgentEvent {
        id: db.last_insert_rowid(),
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
        created_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
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
    fn persists_normalized_agent_events_in_sequence() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("bridge.db")).unwrap();
        db.execute(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/demo','now')",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Kyoto','Task','bridge/task','/tmp/w','idle','now')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('s','w','codex','Codex','working','reported')", []).unwrap();
        let first = crate::agent::normalize_codex_message(
            &json!({"method":"item/agentMessage/delta","params":{"itemId":"m","delta":"hel"}}),
        );
        let second = crate::agent::normalize_codex_message(
            &json!({"method":"item/agentMessage/delta","params":{"itemId":"m","delta":"lo"}}),
        );
        agent_event(&db, "s", &first[0], &json!({"provider":"codex"})).unwrap();
        agent_event(&db, "s", &second[0], &json!({"provider":"codex"})).unwrap();
        let snapshot = state(&db).unwrap();
        assert_eq!(snapshot.agent_events.len(), 2);
        assert_eq!(snapshot.agent_events[0].sequence, 1);
        assert_eq!(snapshot.agent_events[1].text.as_deref(), Some("lo"));
    }
}

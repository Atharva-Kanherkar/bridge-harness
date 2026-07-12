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
    let sessions = query(db, "SELECT id,workspace_id,harness,label,status,started_at,ended_at,context_percent,usage_percent,metric_source FROM sessions ORDER BY rowid", |r| Ok(Session { id:r.get(0)?, workspace_id:r.get(1)?, harness:harness(&r.get::<_,String>(2)?), label:r.get(3)?, status:status(&r.get::<_,String>(4)?), started_at:r.get(5)?, ended_at:r.get(6)?, context_percent:r.get(7)?, usage_percent:r.get(8)?, metric_source:r.get(9)? }))?;
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

#[cfg(test)]
mod tests {
    use super::*;
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
}

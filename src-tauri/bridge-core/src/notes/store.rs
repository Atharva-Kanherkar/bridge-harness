//! `notes.db`: the notes store, its schema, and its one-row-per-device facts.
//!
//! A separate SQLite file with its own connection and schema version, not a
//! migration of `bridge.db`. Notes traffic never waits on the core database
//! lock, the folder exports cleanly, and a later cloud split moves one file.
//!
//! The tables are an index and an append-only log. `page_updates` is the
//! truth: replaying it reproduces every page. `pages` is rebuilt from it (see
//! `service::rebuild_index`), so a damaged index costs a replay, not data.

use std::path::Path;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};

use super::ids::new_id;
use crate::BridgeError;

pub const SCHEMA_VERSION: i64 = 1;
pub const FILE_NAME: &str = "notes.db";

const SCHEMA_V1: &str = "
CREATE TABLE spaces(
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('personal','team')),
    created_at TEXT NOT NULL
);
CREATE TABLE pages(
    id TEXT PRIMARY KEY,
    space_id TEXT NOT NULL REFERENCES spaces(id),
    parent_id TEXT,
    source_id TEXT,
    title TEXT,
    kind TEXT NOT NULL DEFAULT 'note' CHECK(kind IN ('note','idea','task','question')),
    tags TEXT NOT NULL DEFAULT '[]',
    workspace_id TEXT,
    state TEXT NOT NULL CHECK(state IN ('inbox','open','doing','done','archived')),
    session_ids TEXT NOT NULL DEFAULT '[]',
    source_excerpt TEXT,
    path TEXT,
    content_hash TEXT,
    projected_seq INTEGER,
    body TEXT NOT NULL DEFAULT '',
    body_hash TEXT NOT NULL,
    seq INTEGER NOT NULL DEFAULT 0,
    created_by TEXT NOT NULL,
    last_actor TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT
);
CREATE INDEX idx_pages_space_state ON pages(space_id, state, updated_at);
CREATE INDEX idx_pages_parent ON pages(parent_id);
CREATE TABLE page_updates(
    page_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    hlc TEXT NOT NULL,
    actor TEXT NOT NULL,
    device_id TEXT NOT NULL,
    update_bytes BLOB NOT NULL,
    PRIMARY KEY(page_id, seq)
);
CREATE TABLE page_snapshots(
    page_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    state BLOB NOT NULL,
    PRIMARY KEY(page_id, seq)
);
CREATE TABLE structured_views(
    page_id TEXT PRIMARY KEY,
    body_hash TEXT NOT NULL,
    view TEXT NOT NULL,
    harness TEXT,
    model TEXT,
    created_at TEXT NOT NULL
);
CREATE TABLE model_jobs(
    id TEXT PRIMARY KEY,
    space_id TEXT NOT NULL REFERENCES spaces(id),
    page_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('structure','tasks')),
    status TEXT NOT NULL CHECK(status IN ('pending','running','done','failed')),
    attempts INTEGER NOT NULL DEFAULT 0,
    available_at TEXT NOT NULL,
    lease_owner TEXT,
    requested_harness TEXT,
    requested_model TEXT,
    harness TEXT,
    model TEXT,
    outcome TEXT,
    detail TEXT,
    tokens INTEGER NOT NULL DEFAULT 0,
    spend_microusd INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_model_jobs_due ON model_jobs(status, available_at);
CREATE TABLE notes_settings(
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

/// Open (or create) `notes.db` at `path` and bring it to [`SCHEMA_VERSION`].
pub fn open(path: &Path) -> Result<Connection, BridgeError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut connection = Connection::open(path)?;
    connection.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA busy_timeout=2000;
         PRAGMA foreign_keys=ON;
         CREATE TABLE IF NOT EXISTS notes_schema_version(
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL
         );",
    )?;
    migrate(&mut connection)?;
    ensure_personal_space(&connection)?;
    ensure_device_id(&connection)?;
    Ok(connection)
}

fn migrate(connection: &mut Connection) -> Result<(), BridgeError> {
    let current: i64 = connection.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM notes_schema_version",
        [],
        |row| row.get(0),
    )?;
    if current > SCHEMA_VERSION {
        return Err(BridgeError::Invalid(format!(
            "notes.db is at schema {current}, newer than this build understands ({SCHEMA_VERSION})"
        )));
    }
    for version in (current + 1)..=SCHEMA_VERSION {
        let transaction = connection.transaction()?;
        match version {
            1 => transaction.execute_batch(SCHEMA_V1)?,
            _ => unreachable!("every version up to SCHEMA_VERSION has an arm"),
        }
        transaction.execute(
            "INSERT INTO notes_schema_version(version, applied_at) VALUES(?1, ?2)",
            params![version, Utc::now().to_rfc3339()],
        )?;
        transaction.commit()?;
    }
    Ok(())
}

fn ensure_personal_space(connection: &Connection) -> Result<(), BridgeError> {
    connection.execute(
        "INSERT INTO spaces(id, name, kind, created_at)
         SELECT ?1, 'Personal', 'personal', ?2
         WHERE NOT EXISTS (SELECT 1 FROM spaces WHERE kind='personal')",
        params![new_id(), Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

fn ensure_device_id(connection: &Connection) -> Result<(), BridgeError> {
    connection.execute(
        "INSERT OR IGNORE INTO notes_settings(key, value) VALUES('device_id', ?1)",
        params![uuid::Uuid::new_v4().simple().to_string()[..12].to_owned()],
    )?;
    Ok(())
}

/// The one personal space every local page belongs to.
pub fn personal_space(connection: &Connection) -> Result<String, BridgeError> {
    Ok(connection.query_row("SELECT id FROM spaces WHERE kind='personal' ORDER BY created_at LIMIT 1", [], |row| row.get(0))?)
}

/// This install's device id, used in clock stamps and change rows.
pub fn device_id(connection: &Connection) -> Result<String, BridgeError> {
    Ok(get_setting(connection, "device_id")?.unwrap_or_else(|| "local".into()))
}

/// The newest clock stamp this device wrote, to resume the clock after a restart.
pub fn newest_stamp(connection: &Connection, device: &str) -> Result<Option<String>, BridgeError> {
    Ok(connection
        .query_row("SELECT MAX(hlc) FROM page_updates WHERE device_id=?1", params![device], |row| row.get(0))
        .optional()?
        .flatten())
}

pub fn get_setting(connection: &Connection, key: &str) -> Result<Option<String>, BridgeError> {
    Ok(connection
        .query_row("SELECT value FROM notes_settings WHERE key=?1", params![key], |row| row.get(0))
        .optional()?)
}

pub fn set_setting(connection: &Connection, key: &str, value: &str) -> Result<(), BridgeError> {
    connection.execute(
        "INSERT INTO notes_settings(key, value) VALUES(?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, value],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables(connection: &Connection) -> Vec<String> {
        let mut statement = connection
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap();
        statement.query_map([], |row| row.get(0)).unwrap().map(Result::unwrap).collect()
    }

    #[test]
    fn opening_creates_schema_and_one_personal_space() {
        let dir = tempfile::tempdir().unwrap();
        let connection = open(&dir.path().join(FILE_NAME)).unwrap();
        let names = tables(&connection);
        for table in ["spaces", "pages", "page_updates", "page_snapshots", "structured_views", "model_jobs", "notes_settings", "notes_schema_version"] {
            assert!(names.contains(&table.to_string()), "missing {table}: {names:?}");
        }
        let spaces: i64 = connection.query_row("SELECT COUNT(*) FROM spaces WHERE kind='personal'", [], |row| row.get(0)).unwrap();
        assert_eq!(spaces, 1);
        let space = personal_space(&connection).unwrap();
        assert_eq!(uuid::Uuid::parse_str(&space).unwrap().get_version_num(), 7);
        assert_eq!(device_id(&connection).unwrap().len(), 12);
    }

    #[test]
    fn reopening_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let first = open(&path).unwrap();
        let space = personal_space(&first).unwrap();
        let device = device_id(&first).unwrap();
        drop(first);
        let second = open(&path).unwrap();
        assert_eq!(personal_space(&second).unwrap(), space);
        assert_eq!(device_id(&second).unwrap(), device);
        let versions: i64 = second.query_row("SELECT COUNT(*) FROM notes_schema_version", [], |row| row.get(0)).unwrap();
        assert_eq!(versions, SCHEMA_VERSION);
    }
}

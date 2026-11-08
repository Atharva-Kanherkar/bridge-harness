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

pub const SCHEMA_VERSION: i64 = 5;
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

/// v2: page assets (images), stored once per space by content hash.
const SCHEMA_V2: &str = "
CREATE TABLE assets(
    space_id TEXT NOT NULL REFERENCES spaces(id),
    hash TEXT NOT NULL,
    mime TEXT NOT NULL,
    ext TEXT NOT NULL,
    size INTEGER NOT NULL,
    bytes BLOB NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY(space_id, hash)
);
";

/// v3: rebuildable full-text search and the page link graph.
const SCHEMA_V3: &str = "
CREATE VIRTUAL TABLE pages_fts USING fts5(
    page_id UNINDEXED, space_id UNINDEXED, title, body, tags,
    tokenize = 'porter unicode61'
);
CREATE TABLE pages_fts_map(
    fts_rowid INTEGER PRIMARY KEY,
    page_id TEXT NOT NULL UNIQUE
);
ALTER TABLE pages ADD COLUMN link_key TEXT;
CREATE INDEX idx_pages_link_key ON pages(space_id, link_key);
CREATE TABLE page_links(
    space_id TEXT NOT NULL REFERENCES spaces(id),
    from_page_id TEXT NOT NULL,
    to_page_id TEXT,
    target_title TEXT NOT NULL,
    target_key TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('wiki', 'page')),
    excerpt TEXT NOT NULL,
    PRIMARY KEY(space_id, from_page_id, kind, target_title)
);
CREATE INDEX idx_page_links_to ON page_links(space_id, to_page_id);
CREATE INDEX idx_page_links_key ON page_links(space_id, target_key);
";

/// v4: database pages, their properties, the values index and the system
/// database. `pages` is rebuilt because SQLite cannot change a `CHECK` in
/// place and the new kind has to be accepted.
const SCHEMA_V4: &str = "
CREATE TABLE pages_v4(
    id TEXT PRIMARY KEY,
    space_id TEXT NOT NULL REFERENCES spaces(id),
    parent_id TEXT,
    source_id TEXT,
    title TEXT,
    kind TEXT NOT NULL DEFAULT 'note' CHECK(kind IN ('note','idea','task','question','database')),
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
    deleted_at TEXT,
    link_key TEXT
);
INSERT INTO pages_v4(id, space_id, parent_id, source_id, title, kind, tags, workspace_id, state, session_ids, source_excerpt, path, content_hash, projected_seq, body, body_hash, seq, created_by, last_actor, created_at, updated_at, deleted_at, link_key)
SELECT id, space_id, parent_id, source_id, title, kind, tags, workspace_id, state, session_ids, source_excerpt, path, content_hash, projected_seq, body, body_hash, seq, created_by, last_actor, created_at, updated_at, deleted_at, link_key
FROM pages;
DROP TABLE pages;
ALTER TABLE pages_v4 RENAME TO pages;
CREATE INDEX idx_pages_space_state ON pages(space_id, state, updated_at);
CREATE INDEX idx_pages_parent ON pages(parent_id);
CREATE INDEX idx_pages_link_key ON pages(space_id, link_key);
ALTER TABLE spaces ADD COLUMN system_database_id TEXT;
CREATE TABLE database_props(
    space_id TEXT NOT NULL REFERENCES spaces(id),
    database_id TEXT NOT NULL,
    prop_id TEXT NOT NULL,
    name TEXT NOT NULL,
    type TEXT NOT NULL,
    def TEXT NOT NULL,
    PRIMARY KEY(space_id, database_id, prop_id)
);
CREATE TABLE page_values(
    space_id TEXT NOT NULL REFERENCES spaces(id),
    database_id TEXT NOT NULL,
    page_id TEXT NOT NULL,
    prop_id TEXT NOT NULL,
    item INTEGER NOT NULL,
    text_v TEXT,
    num_v REAL,
    date_v TEXT,
    PRIMARY KEY(space_id, page_id, prop_id, item)
);
CREATE INDEX idx_page_values_num ON page_values(space_id, database_id, prop_id, num_v);
CREATE INDEX idx_page_values_text ON page_values(space_id, database_id, prop_id, text_v);
CREATE INDEX idx_page_values_date ON page_values(space_id, database_id, prop_id, date_v);
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

/// v5: notices about a row's file the importer could not read (a value that
/// does not parse for its property), shown on the row page. Not part of the
/// change log: `rebuild_index` clears them and the next import records them
/// again.
const SCHEMA_V5: &str = "
CREATE TABLE page_notices(
    space_id TEXT NOT NULL REFERENCES spaces(id),
    page_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    property TEXT NOT NULL,
    raw TEXT NOT NULL,
    expected TEXT NOT NULL,
    seq INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY(space_id, page_id, kind, property)
);
";

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
            2 => transaction.execute_batch(SCHEMA_V2)?,
            3 => {
                transaction.execute_batch(SCHEMA_V3)?;
                super::links::backfill(&transaction)?;
            }
            4 => transaction.execute_batch(SCHEMA_V4)?,
            5 => transaction.execute_batch(SCHEMA_V5)?,
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
    fn v1_and_v2_migrate_to_v4_with_fts_and_link_backfill() {
        for version in [1, 2] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(FILE_NAME);
            let db = Connection::open(&path).unwrap();
            db.execute_batch(SCHEMA_V1).unwrap();
            if version == 2 { db.execute_batch(SCHEMA_V2).unwrap(); }
            db.execute_batch("CREATE TABLE notes_schema_version(version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL)").unwrap();
            db.execute("INSERT INTO notes_schema_version VALUES(?1, 'then')", [version]).unwrap();
            db.execute("INSERT INTO spaces VALUES('space', 'Personal', 'personal', 'then')", []).unwrap();
            for (id, title, body) in [("source", "Source", "[[Target]]"), ("target", "Target", "running"), ("deleted", "Deleted", "[[Target]]")] {
                db.execute("INSERT INTO pages(id,space_id,title,body,body_hash,state,created_by,last_actor,created_at,updated_at,deleted_at) VALUES(?1,'space',?2,?3,'hash','open','person','person','then','then',?4)", params![id, title, body, (id == "deleted").then_some("then")]).unwrap();
            }
            drop(db);
            let db = open(&path).unwrap();
            let schema: i64 = db.query_row("SELECT max(version) FROM notes_schema_version", [], |r| r.get(0)).unwrap();
            assert_eq!(schema, SCHEMA_VERSION);
            let hits: String = db.query_row("SELECT page_id FROM pages_fts WHERE pages_fts MATCH 'runs'", [], |r| r.get(0)).unwrap();
            assert_eq!(hits, "target");
            let backlinks = super::super::links::read_backlinks(&db, "space", "target").unwrap();
            assert_eq!(backlinks.len(), 1);
            assert_eq!(backlinks[0].page_id, "source");
            assert!(tables(&db).contains(&"assets".into()));
            assert_eq!(db.query_row("SELECT count(*) FROM pages", [], |r| r.get::<_, i64>(0)).unwrap(), 3);
        }
    }

    /// A v3 file with every kind of row, so the `pages` rebuild has something
    /// to lose.
    fn v3_fixture(path: &Path) -> Connection {
        let db = Connection::open(path).unwrap();
        db.execute_batch(SCHEMA_V1).unwrap();
        db.execute_batch(SCHEMA_V2).unwrap();
        db.execute_batch(SCHEMA_V3).unwrap();
        db.execute_batch("CREATE TABLE notes_schema_version(version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL)").unwrap();
        db.execute("INSERT INTO notes_schema_version VALUES(3, 'then')", []).unwrap();
        db.execute("INSERT INTO spaces VALUES('space', 'Personal', 'personal', 'then')", []).unwrap();
        for (id, kind, title, body, deleted) in [("plan", "note", "Plan", "ship [[Target]]", false), ("task", "task", "Buy domain", "- [ ] domain", false), ("gone", "idea", "Gone", "old", true)] {
            db.execute(
                "INSERT INTO pages(id,space_id,parent_id,title,kind,state,body,body_hash,seq,created_by,last_actor,created_at,updated_at,deleted_at,link_key)
                 VALUES(?1,'space',?2,?3,?4,'open',?5,'h',7,'person','person','then','then',?6,lower(?3))",
                params![id, (id == "task").then_some("plan"), title, kind, body, deleted.then_some("then")],
            ).unwrap();
            db.execute("INSERT INTO page_updates(page_id, seq, hlc, actor, device_id, update_bytes) VALUES(?1, 1, '1-0-dev', 'person', 'dev', x'00')", [id]).unwrap();
        }
        db.execute("INSERT INTO page_links(space_id, from_page_id, to_page_id, target_title, target_key, kind, excerpt) VALUES('space','plan',NULL,'Target','target','wiki','ship')", []).unwrap();
        db.execute("INSERT INTO assets(space_id, hash, mime, ext, size, bytes, created_at) VALUES('space','h1','image/png','png',3,x'010203','then')", []).unwrap();
        db.execute("INSERT INTO pages_fts(page_id, space_id, title, body, tags) VALUES('plan','space','Plan','ship [[Target]]','[]')", []).unwrap();
        db.execute("INSERT INTO pages_fts_map(fts_rowid, page_id) VALUES(last_insert_rowid(),'plan')", []).unwrap();
        db
    }

    #[test]
    fn v3_migrates_to_v4_and_keeps_every_row_and_index() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let fixture = v3_fixture(&path);
        drop(fixture);
        let db = open(&path).unwrap();
        assert_eq!(db.query_row("SELECT max(version) FROM notes_schema_version", [], |r| r.get::<_, i64>(0)).unwrap(), SCHEMA_VERSION);
        assert_eq!(db.query_row("SELECT count(*) FROM pages", [], |r| r.get::<_, i64>(0)).unwrap(), 3);
        assert_eq!(db.query_row("SELECT count(*) FROM page_updates", [], |r| r.get::<_, i64>(0)).unwrap(), 3);
        assert_eq!(db.query_row("SELECT count(*) FROM page_links", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(db.query_row("SELECT count(*) FROM assets", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(db.query_row("SELECT count(*) FROM pages_fts_map", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        let (title, link_key, seq, deleted): (String, String, i64, Option<String>) = db
            .query_row("SELECT title, link_key, seq, deleted_at FROM pages WHERE id='task'", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap();
        assert_eq!((title.as_str(), link_key.as_str(), seq, deleted), ("Buy domain", "buy domain", 7, None));
        assert_eq!(db.query_row("SELECT parent_id FROM pages WHERE id='task'", [], |r| r.get::<_, Option<String>>(0)).unwrap().as_deref(), Some("plan"));
        let names = tables(&db);
        for table in ["database_props", "page_values"] {
            assert!(names.contains(&table.to_string()), "missing {table}");
        }
        for index in ["idx_pages_space_state", "idx_pages_parent", "idx_pages_link_key", "idx_page_values_num", "idx_page_values_text", "idx_page_values_date"] {
            let found: i64 = db.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name=?1", [index], |r| r.get(0)).unwrap();
            assert_eq!(found, 1, "missing index {index}");
        }
        // The rebuilt table accepts the new kind and still refuses the others.
        db.execute("INSERT INTO pages(id,space_id,kind,state,body_hash,created_by,last_actor,created_at,updated_at) VALUES('db','space','database','open','h','person','person','then','then')", []).unwrap();
        assert!(db.execute("INSERT INTO pages(id,space_id,kind,state,body_hash,created_by,last_actor,created_at,updated_at) VALUES('bad','space','essay','open','h','person','person','then','then')", []).is_err());
        assert_eq!(db.query_row("SELECT count(*) FROM spaces WHERE system_database_id IS NULL", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn migrates_a_v4_store_to_page_notices() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        drop(v3_fixture(&path));
        let db = open(&path).unwrap();
        assert_eq!(db.query_row("SELECT max(version) FROM notes_schema_version", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
        assert!(tables(&db).contains(&"page_notices".to_string()));
        assert_eq!(db.query_row("SELECT count(*) FROM pages", [], |r| r.get::<_, i64>(0)).unwrap(), 3, "every page kept");
        db.execute("INSERT INTO page_notices(space_id, page_id, kind, property, raw, expected, seq, created_at) VALUES('space','plan','unreadableValue','p','three','number',1,'now')", []).unwrap();
        // One notice per page, kind and property.
        assert!(db.execute("INSERT INTO page_notices(space_id, page_id, kind, property, raw, expected, seq, created_at) VALUES('space','plan','unreadableValue','p','four','number',2,'now')", []).is_err());
    }

    #[test]
    fn a_newer_schema_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE notes_schema_version(version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL)").unwrap();
        db.execute("INSERT INTO notes_schema_version VALUES(?1, 'later')", [SCHEMA_VERSION + 1]).unwrap();
        drop(db);
        let error = open(&path).unwrap_err().to_string();
        assert!(error.contains("newer than this build understands"), "{error}");
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

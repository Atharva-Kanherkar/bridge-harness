//! A fixture OpenCode store for tests, built with the columns of OpenCode's
//! own schema (opencode 2.1.1). Its account and credential tables hold
//! canaries: nothing the importer outputs may ever contain them.

use rusqlite::{params, Connection};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const CANARY: &str = "CANARY-opencode-secret";

const SCHEMA: &str = "
CREATE TABLE account (id text PRIMARY KEY, email text NOT NULL, url text NOT NULL, access_token text NOT NULL,
    refresh_token text NOT NULL, token_expiry integer, time_created integer NOT NULL, time_updated integer NOT NULL);
CREATE TABLE credential (id text PRIMARY KEY, integration_id text, label text NOT NULL, value text NOT NULL,
    connector_id text, method_id text, active integer, time_created integer NOT NULL, time_updated integer NOT NULL);
CREATE TABLE control_account (email text NOT NULL, url text NOT NULL, access_token text NOT NULL, refresh_token text NOT NULL,
    token_expiry integer, active integer NOT NULL, time_created integer NOT NULL, time_updated integer NOT NULL,
    PRIMARY KEY(email, url));
CREATE TABLE project (id text PRIMARY KEY, worktree text NOT NULL, vcs text, name text, time_created integer NOT NULL,
    time_updated integer NOT NULL, sandboxes text NOT NULL);
CREATE TABLE session (id text PRIMARY KEY, project_id text NOT NULL, workspace_id text, parent_id text, slug text NOT NULL,
    directory text NOT NULL, path text, title text NOT NULL, version text NOT NULL, share_url text, metadata text,
    agent text, model text, time_created integer NOT NULL, time_updated integer NOT NULL, time_compacting integer,
    time_archived integer);
CREATE TABLE message (id text PRIMARY KEY, session_id text NOT NULL, time_created integer NOT NULL,
    time_updated integer NOT NULL, data text NOT NULL);
CREATE TABLE part (id text PRIMARY KEY, message_id text NOT NULL, session_id text NOT NULL, time_created integer NOT NULL,
    time_updated integer NOT NULL, data text NOT NULL);
CREATE TABLE session_v2 (id text PRIMARY KEY, project_id text NOT NULL, workspace_id text, parent_id text,
    fork_session_id text, fork_boundary text, slug text NOT NULL, directory text NOT NULL, path text, title text,
    version text NOT NULL, share_url text, metadata text, agent text, model text, time_created integer NOT NULL,
    time_updated integer NOT NULL, time_compacting integer, time_archived integer);
CREATE TABLE session_message (id text PRIMARY KEY, session_id text NOT NULL, type text NOT NULL, seq integer NOT NULL,
    time_created integer NOT NULL, time_updated integer NOT NULL, data text NOT NULL);
CREATE UNIQUE INDEX session_message_session_seq_idx ON session_message (session_id, seq);
";

/// One OpenCode data folder holding a fixture store.
pub struct Fixture {
    /// The data folder (`<data home>/opencode`).
    pub home: PathBuf,
    pub store: PathBuf,
    conn: Connection,
}

impl Fixture {
    /// A store with both generations of tables, under `<data_home>/opencode`.
    pub fn new(data_home: &Path) -> Self {
        Self::with_schema(data_home, SCHEMA)
    }

    /// A 1.x store: no `session_v2` or `session_message`.
    pub fn v1_only(data_home: &Path) -> Self {
        let v1: String = SCHEMA
            .split(';')
            .filter(|statement| !statement.contains("session_v2") && !statement.contains("session_message"))
            .collect::<Vec<_>>()
            .join(";");
        Self::with_schema(data_home, &v1)
    }

    fn with_schema(data_home: &Path, schema: &str) -> Self {
        let home = data_home.join("opencode");
        std::fs::create_dir_all(&home).unwrap();
        let store = home.join("opencode.db");
        let conn = Connection::open(&store).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        conn.execute_batch(schema).unwrap();
        conn.execute(
            "INSERT INTO account VALUES('a1','me@example.com','https://opencode.ai',?1,?1,NULL,1,1)",
            params![CANARY],
        )
        .unwrap();
        conn.execute("INSERT INTO credential VALUES('c1',NULL,'key',?1,NULL,NULL,1,1,1)", params![CANARY]).unwrap();
        conn.execute("INSERT INTO control_account VALUES('me@example.com','u',?1,?1,NULL,1,1,1)", params![CANARY]).unwrap();
        Self { home: home.canonicalize().unwrap(), store: store.canonicalize().unwrap(), conn }
    }

    /// A 2.x session.
    pub fn session(&self, id: &str, parent: Option<&str>, directory: &str, title: Option<&str>, created_ms: i64) {
        self.conn
            .execute(
                "INSERT INTO session_v2(id,project_id,parent_id,slug,directory,title,version,model,time_created,time_updated)
                 VALUES(?1,'p',?2,?1,?3,?4,'2.1.1',NULL,?5,?5)",
                params![id, parent, directory, title, created_ms],
            )
            .unwrap();
    }

    pub fn archive(&self, id: &str) {
        self.conn.execute("UPDATE session_v2 SET time_archived=1 WHERE id=?1", params![id]).unwrap();
        self.conn.execute("UPDATE session SET time_archived=1 WHERE id=?1", params![id]).unwrap();
    }

    /// A 2.x row: `user`, `assistant`, `synthetic`, `compaction`, ...
    pub fn row(&self, session: &str, seq: i64, kind: &str, data: &Value) {
        self.raw_row(session, seq, kind, &data.to_string());
    }

    pub fn raw_row(&self, session: &str, seq: i64, kind: &str, data: &str) {
        self.conn
            .execute(
                "INSERT INTO session_message(id,session_id,type,seq,time_created,time_updated,data) VALUES(?1,?2,?3,?4,?5,?5,?6)",
                params![format!("{session}-m{seq}"), session, kind, seq, 1_790_000_000_000_i64 + seq, data],
            )
            .unwrap();
    }

    /// A 1.x session.
    pub fn v1_session(&self, id: &str, parent: Option<&str>, directory: &str, title: &str, created_ms: i64) {
        self.conn
            .execute(
                "INSERT INTO session(id,project_id,parent_id,slug,directory,title,version,time_created,time_updated)
                 VALUES(?1,'p',?2,?1,?3,?4,'1.18.31',?5,?5)",
                params![id, parent, directory, title, created_ms],
            )
            .unwrap();
    }

    /// A 1.x message and its parts, in order.
    pub fn v1_message(&self, session: &str, id: &str, created_ms: i64, message: &Value, parts: &[Value]) {
        self.conn
            .execute(
                "INSERT INTO message(id,session_id,time_created,time_updated,data) VALUES(?1,?2,?3,?3,?4)",
                params![id, session, created_ms, message.to_string()],
            )
            .unwrap();
        for (index, part) in parts.iter().enumerate() {
            self.conn
                .execute(
                    "INSERT INTO part(id,message_id,session_id,time_created,time_updated,data) VALUES(?1,?2,?3,?4,?4,?5)",
                    params![format!("{id}-p{index:03}"), id, session, created_ms, part.to_string()],
                )
                .unwrap();
        }
    }

    /// Delete a session from the store, as OpenCode does.
    pub fn delete(&self, id: &str) {
        self.conn.execute("DELETE FROM session_message WHERE session_id=?1", params![id]).ok();
        self.conn.execute("DELETE FROM session_v2 WHERE id=?1", params![id]).ok();
        self.conn.execute("DELETE FROM session WHERE id=?1", params![id]).ok();
    }
}

/// A short user row.
pub fn said(text: &str) -> Value {
    serde_json::json!({ "time": { "created": 1 }, "text": text })
}

/// A short assistant row with one text item.
pub fn replied(text: &str) -> Value {
    serde_json::json!({
        "time": { "created": 1 },
        "agent": "build",
        "model": { "id": "big-pickle", "providerID": "opencode", "variant": "default" },
        "content": [{ "type": "text", "text": text }],
        "finish": "stop"
    })
}

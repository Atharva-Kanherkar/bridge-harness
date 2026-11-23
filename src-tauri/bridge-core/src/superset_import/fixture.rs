//! A Superset app home for tests, on the schemas rebuilt with Superset's
//! own migrations (`testing/fixtures/import/superset/`). Rows go into the
//! columns that head has, shaped as the 2.0.11 host writes them. Every place
//! Bridge must never read holds a canary: agent configurations' values, the
//! mobile tables, and the token and sign-in files.

use rusqlite::types::Value as Sql;
use rusqlite::{params_from_iter, Connection};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const CANARY: &str = "CANARY-superset-secret";
pub const ORG: &str = "7f3c2b1a-4d5e-4f60-8a9b-0c1d2e3f4a5b";
/// The rebuilt heads, oldest first.
pub const HOST_HEADS: &[i64] = &[0, 6, 17, 20, 26, 33, 36, 38];
pub const CHAT_HEADS: &[i64] = &[0, 1, 3, 4, 5];

pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testing/fixtures/import/superset")
}

fn create(path: &Path, name: &str, head: i64) -> Connection {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let conn = Connection::open(path).unwrap();
    // Test rows (canaries) need not point at real rows.
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=OFF;").unwrap();
    conn.execute_batch(&std::fs::read_to_string(fixtures_dir().join(format!("{name}-{head}.sql"))).unwrap()).unwrap();
    for when in std::fs::read_to_string(fixtures_dir().join(format!("{name}-{head}.txt"))).unwrap().lines() {
        conn.execute("INSERT INTO __drizzle_migrations(hash, created_at) VALUES('h', ?1)", [when.parse::<i64>().unwrap()]).unwrap();
    }
    conn
}

/// A text value.
pub fn t(value: impl Into<String>) -> Sql {
    Sql::Text(value.into())
}

/// Insert into the columns the table has; NOT NULL columns with no default
/// that the row leaves out get an empty value of their type.
pub fn insert(conn: &Connection, table: &str, values: &[(&str, Sql)]) {
    let columns: Vec<(String, String, bool, bool)> = conn
        .prepare(&format!("SELECT name, type, \"notnull\", dflt_value IS NOT NULL FROM pragma_table_info('{table}')"))
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut names = Vec::new();
    let mut cells = Vec::new();
    for (name, kind, not_null, has_default) in &columns {
        if let Some((_, value)) = values.iter().find(|(column, _)| column == name) {
            names.push(format!("\"{name}\""));
            cells.push(value.clone());
        } else if *not_null && !has_default {
            names.push(format!("\"{name}\""));
            cells.push(if kind.to_ascii_lowercase().contains("int") { Sql::Integer(0) } else { t("") });
        }
    }
    let marks = vec!["?"; names.len()].join(", ");
    conn.execute(&format!("INSERT INTO {table}({}) VALUES({marks})", names.join(", ")), params_from_iter(cells)).unwrap();
}

/// A workspace as a test describes it.
#[derive(Debug, Clone)]
pub struct WorkspaceRow<'a> {
    pub id: &'a str,
    pub project: Option<&'a str>,
    pub path: &'a Path,
    pub branch: &'a str,
    pub name: &'a str,
    pub kind: &'a str,
    pub archived: bool,
}

/// A chat as a test describes it.
#[derive(Debug, Clone)]
pub struct ChatRow<'a> {
    pub id: &'a str,
    pub workspace: &'a str,
    pub harness: &'a str,
    pub native: Option<&'a str>,
    pub epoch: &'a str,
    pub title: Option<&'a str>,
}

impl Default for ChatRow<'_> {
    fn default() -> Self {
        Self { id: "", workspace: "", harness: "opencode-acp", native: None, epoch: "e1", title: None }
    }
}

pub struct Fixture {
    pub home: PathBuf,
    pub folder: PathBuf,
    pub host: Connection,
    pub chat: Option<Connection>,
}

impl Fixture {
    /// An app home at `home` with one organization at these heads (no
    /// `chat.db` when `chat_head` is none).
    pub fn at(home: &Path, host_head: i64, chat_head: Option<i64>) -> Self {
        let folder = home.join("host").join(ORG);
        let host = create(&folder.join(super::store::HOST_DB), "host", host_head);
        let chat = chat_head.map(|head| create(&folder.join(super::store::CHAT_DB), "chat", head));
        let fixture = Self { home: home.to_path_buf(), folder, host, chat };
        fixture.secrets();
        fixture
    }

    /// The newest heads.
    pub fn newest(home: &Path) -> Self {
        Self::at(home, *HOST_HEADS.last().unwrap(), Some(*CHAT_HEADS.last().unwrap()))
    }

    fn has_table(conn: &Connection, table: &str) -> bool {
        conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1", [table], |row| row.get::<_, i64>(0)).unwrap() > 0
    }

    /// Canaries everywhere Bridge must not read, the files made unreadable.
    fn secrets(&self) {
        use std::os::unix::fs::PermissionsExt;
        let plant = |path: PathBuf| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, CANARY).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        };
        for name in ["auth-token.enc", "config.json", "terminal-host.token", "cloud-workspace-tickets.json", "plugin-mcp-sessions.json", "local.db"] {
            plant(self.home.join(name));
        }
        plant(self.home.join("state/default-claude-config-dir"));
        plant(self.home.join("agent/mcp.json"));
        plant(self.folder.join("manifest.json"));
        if Self::has_table(&self.host, "host_agent_configs") {
            self.config("cfg-seeded", Some("claude"), "Claude", &json!({}));
        }
        for table in ["mobile_owned_devices", "mobile_last_devices"] {
            if Self::has_table(&self.host, table) {
                let columns: Vec<String> = self.host.prepare(&format!("SELECT name FROM pragma_table_info('{table}')")).unwrap().query_map([], |row| row.get(0)).unwrap().map(Result::unwrap).collect();
                let values: Vec<(&str, Sql)> = columns.iter().map(|column| (column.as_str(), t(CANARY))).collect();
                insert(&self.host, table, &values);
            }
        }
    }

    pub fn project(&self, id: &str, root: &Path, name: &str) {
        insert(&self.host, "projects", &[("id", t(id)), ("repo_path", t(root.to_string_lossy())), ("name", t(name)), ("created_at", Sql::Integer(1)), ("updated_at", Sql::Integer(1))]);
    }

    pub fn delete_project(&self, id: &str) {
        self.host.execute("UPDATE projects SET deleted_at = 5 WHERE id = ?1", [id]).unwrap();
    }

    pub fn workspace(&self, row: &WorkspaceRow<'_>) {
        let mut values = vec![
            ("id", t(row.id)),
            ("worktree_path", t(row.path.to_string_lossy())),
            ("branch", t(row.branch)),
            ("name", t(row.name)),
            ("type", t(row.kind)),
            ("created_at", Sql::Integer(1)),
            ("updated_at", Sql::Integer(1)),
        ];
        values.push(("project_id", row.project.map_or(Sql::Null, t)));
        if row.archived {
            values.push(("archived_at", Sql::Integer(1_795_427_138_800)));
            values.push(("archive_reason", t("deleted")));
        }
        insert(&self.host, "workspaces", &values);
    }

    pub fn binding(&self, terminal: &str, workspace: &str, agent: &str, session: Option<&str>, transcript: Option<&Path>) {
        insert(&self.host, "terminal_sessions", &[("id", t(terminal)), ("origin_workspace_id", t(workspace)), ("created_at", Sql::Integer(1))]);
        insert(
            &self.host,
            "terminal_agent_bindings",
            &[
                ("terminal_id", t(terminal)),
                ("workspace_id", t(workspace)),
                ("agent_id", t(agent)),
                ("agent_session_id", session.map_or(Sql::Null, t)),
                ("transcript_path", transcript.map_or(Sql::Null, |path| t(path.to_string_lossy()))),
                ("started_at", Sql::Integer(1)),
                ("last_event_at", Sql::Integer(1)),
            ],
        );
    }

    /// An agent configuration; its environment's values are what Bridge
    /// must never read.
    pub fn config(&self, id: &str, preset: Option<&str>, label: &str, env: &Value) {
        insert(
            &self.host,
            "host_agent_configs",
            &[
                ("id", t(id)),
                ("preset_id", preset.map_or(Sql::Null, t)),
                ("label", t(label)),
                ("command", t("claude")),
                ("args_json", t("[\"--verbose\"]")),
                ("prompt_transport", t("argv")),
                ("prompt_args_json", t("[]")),
                ("env_json", t(env.to_string())),
                ("display_order", Sql::Integer(1)),
                ("created_at", Sql::Integer(1)),
                ("updated_at", Sql::Integer(1)),
            ],
        );
    }

    pub fn tag(&self, workspace: &str, tag: &str) {
        insert(&self.host, "workspace_tags", &[("workspace_id", t(workspace)), ("tag", t(tag)), ("created_at", Sql::Integer(1))]);
    }

    pub fn chat_conn(&self) -> &Connection {
        self.chat.as_ref().expect("this fixture has a chat database")
    }

    pub fn chat(&self, row: &ChatRow<'_>) {
        insert(
            self.chat_conn(),
            "chat_sessions_local",
            &[
                ("session_id", t(row.id)),
                ("scope_id", t(row.workspace)),
                ("harness", t(row.harness)),
                ("harness_session_id", row.native.map_or(Sql::Null, t)),
                ("epoch", t(row.epoch)),
                ("status", t("idle")),
                ("title", row.title.map_or(Sql::Null, t)),
                ("updated_at", Sql::Integer(1_795_426_792_536)),
            ],
        );
    }

    /// Journal events for a chat at `epoch`, numbered in order.
    pub fn events(&self, session: &str, epoch: &str, events: &[Value]) {
        let conn = self.chat_conn();
        let start: i64 = conn.query_row("SELECT COALESCE(MAX(seq), 0) FROM chat_journal WHERE session_id=?1 AND epoch=?2", [session, epoch], |row| row.get(0)).unwrap();
        for (index, event) in events.iter().enumerate() {
            insert(
                conn,
                "chat_journal",
                &[("session_id", t(session)), ("epoch", t(epoch)), ("seq", Sql::Integer(start + index as i64 + 1)), ("ts", Sql::Integer(1_795_426_738_129)), ("event_json", t(event.to_string()))],
            );
        }
    }
}

/// Journal events, as the 2.0.11 host writes them.
pub fn item(item: Value) -> Value {
    json!({ "type": "item", "item": item, "turnId": "turn-1" })
}

pub fn user(id: &str, text: &str) -> Value {
    item(json!({ "id": id, "startedAtMs": 1_795_427_148_315_i64, "kind": "user_message", "clientId": "superset-sessions:1", "content": [{ "type": "text", "text": text }] }))
}

pub fn agent(id: &str, text: &str) -> Value {
    item(json!({ "id": id, "startedAtMs": 1_795_427_152_979_i64, "kind": "agent_message", "text": text, "model": "opencode/Big Pickle" }))
}

pub fn reasoning(id: &str, text: &str) -> Value {
    item(json!({ "id": id, "startedAtMs": 1_795_427_152_000_i64, "kind": "reasoning", "text": text }))
}

pub fn tool(id: &str, name: &str, input: Value, output: &str, status: &str) -> Value {
    item(json!({ "id": id, "startedAtMs": 1_795_427_152_500_i64, "kind": "tool_call", "title": name, "toolKind": "read", "toolName": name, "status": status,
        "content": [{ "type": "text", "text": output }], "rawInput": input }))
}

pub fn session_event(status: &str) -> Value {
    json!({ "type": "session", "session": { "status": status, "harness": "opencode-acp" } })
}

pub fn turn(status: &str) -> Value {
    json!({ "type": "turn", "turn": { "id": "turn-1", "status": status, "startedAtMs": 1 } })
}

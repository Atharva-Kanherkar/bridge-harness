//! A fixture T3 Code database for tests, built from one of the real schemas
//! rebuilt with T3 Code's own migrations (`testing/fixtures/import/t3code/`).
//! Rows go into the columns that version has, shaped as T3 Code writes them
//! (v1 rows before migration 55, v2 rows from it). Its auth, scheduled-task
//! and peer-link tables hold canaries no output may contain.

use rusqlite::types::Value;
use rusqlite::{params_from_iter, Connection};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const CANARY: &str = "CANARY-t3code-secret";

/// The real schema heads there are fixtures for, oldest first.
pub const VERSIONS: &[i64] = &[5, 17, 23, 36, 43, 48, 54, 55, 63, 76, 87, 95, 102, 119];

/// A text value.
pub fn t(value: impl Into<String>) -> Value {
    Value::Text(value.into())
}

pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testing/fixtures/import/t3code")
}

pub fn schema(version: i64) -> String {
    std::fs::read_to_string(fixtures_dir().join(format!("schema-{version}.sql"))).unwrap()
}

pub fn migrations(version: i64) -> Vec<(i64, String)> {
    std::fs::read_to_string(fixtures_dir().join(format!("migrations-{version}.txt")))
        .unwrap()
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(number, name)| (number.parse().unwrap(), name.to_owned()))
        .collect()
}

pub struct Fixture {
    pub path: PathBuf,
    pub conn: Connection,
    pub version: i64,
}

/// A thread as a test describes it; `None` leaves a field empty.
#[derive(Debug, Clone, Default)]
pub struct ThreadRow<'a> {
    pub id: &'a str,
    pub project: &'a str,
    pub title: &'a str,
    pub branch: Option<&'a str>,
    pub worktree: Option<&'a Path>,
    pub archived: bool,
    pub deleted: bool,
    pub pinned: bool,
    /// The parent thread, for a subagent's thread (v2 only).
    pub parent: Option<&'a str>,
    /// T3 Code's driver id, e.g. `claudeAgent`.
    pub driver: &'a str,
    /// The agent's own id for the session.
    pub native: Option<&'a str>,
    pub updated_at: &'a str,
}

/// Column name, declared type, NOT NULL, has a default.
type Columns = Vec<(String, String, bool, bool)>;

impl Fixture {
    /// `<dir>/userdata/statev2.sqlite` (v2) or `state.sqlite` (v1).
    pub fn in_base(base: &Path, version: i64) -> Self {
        let file = if version >= super::store::FIRST_V2_MIGRATION { "statev2.sqlite" } else { "state.sqlite" };
        Self::at(&base.join("userdata").join(file), version)
    }

    pub fn at(path: &Path, version: i64) -> Self {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let conn = Connection::open(path).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        conn.execute_batch(&schema(version)).unwrap();
        for (number, name) in migrations(version) {
            conn.execute("INSERT INTO effect_sql_migrations(migration_id,name) VALUES(?1,?2)", rusqlite::params![number, name]).unwrap();
        }
        let fixture = Self { path: path.to_path_buf(), conn, version };
        for table in ["auth_sessions", "auth_pairing_links", "scheduled_tasks", "peer_environment_links"] {
            fixture.fill(table, &[], CANARY);
        }
        // Workflow tokens and payloads (110 on): every column a canary.
        for table in ["workflow_hooks", "workflow_waits", "workflow_mail", "workflow_store", "issue_tracker_accounts"] {
            let columns = fixture.columns(table);
            let values: Vec<(&str, Value)> = columns
                .iter()
                .map(|(name, kind, ..)| (name.as_str(), if kind.to_ascii_uppercase().contains("INT") { Value::Integer(0) } else { t(CANARY) }))
                .collect();
            fixture.fill(table, &values, CANARY);
        }
        fixture
    }

    pub fn v2(&self) -> bool {
        self.version >= super::store::FIRST_V2_MIGRATION
    }

    fn columns(&self, table: &str) -> Columns {
        let mut statement = self
            .conn
            .prepare(&format!("SELECT name, type, \"notnull\", dflt_value IS NOT NULL FROM pragma_table_info('{table}')"))
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get::<_, i64>(2)? != 0, row.get::<_, i64>(3)? != 0)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    pub fn has_table(&self, table: &str) -> bool {
        !self.columns(table).is_empty()
    }

    pub fn has(&self, table: &str, column: &str) -> bool {
        self.columns(table).iter().any(|(name, ..)| name == column)
    }

    /// Insert into the columns this version has; others are dropped, and a
    /// required column with no value gets `filler` (or 0 for a number).
    pub fn fill(&self, table: &str, values: &[(&str, Value)], filler: &str) {
        self.try_fill(table, values, filler).unwrap();
    }

    pub fn try_fill(&self, table: &str, values: &[(&str, Value)], filler: &str) -> rusqlite::Result<()> {
        let columns = self.columns(table);
        if columns.is_empty() {
            return Ok(());
        }
        let given: HashMap<&str, &Value> = values.iter().map(|(column, value)| (*column, value)).collect();
        let mut kept: Vec<(String, Value)> = Vec::new();
        for (name, kind, not_null, has_default) in &columns {
            if let Some(value) = given.get(name.as_str()) {
                kept.push((name.clone(), (*value).clone()));
            } else if *not_null && !has_default {
                let numeric = kind.to_ascii_uppercase().contains("INT");
                let value = if numeric { Value::Integer(0) } else if name.ends_with("_json") { t("{}") } else { t(filler) };
                kept.push((name.clone(), value));
            }
        }
        let sql = format!(
            "INSERT INTO {table}({}) VALUES({})",
            kept.iter().map(|(column, _)| format!("\"{column}\"")).collect::<Vec<_>>().join(","),
            vec!["?"; kept.len()].join(",")
        );
        self.conn.execute(&sql, params_from_iter(kept.into_iter().map(|(_, value)| value)))?;
        Ok(())
    }

    pub fn insert(&self, table: &str, values: &[(&str, Value)]) {
        self.fill(table, values, "x");
    }

    pub fn project(&self, id: &str, root: &Path, title: &str) {
        self.insert("projection_projects", &[
            ("project_id", t(id)),
            ("title", t(title)),
            ("workspace_root", t(root.to_string_lossy().into_owned())),
            ("scripts_json", t(r#"[{"id":"setup","name":"Install","command":"bun install","icon":"configure","runOnWorktreeCreate":true},{"id":"dev","name":"Dev","command":"bun dev","icon":"play","runOnWorktreeCreate":false}]"#)),
            ("created_at", t("2026-11-01T00:00:00.000Z")),
            ("updated_at", t("2026-11-01T00:00:00.000Z")),
        ]);
    }

    pub fn delete_project(&self, id: &str) {
        self.conn.execute("UPDATE projection_projects SET deleted_at='2026-11-02T00:00:00.000Z' WHERE project_id=?1", [id]).unwrap();
    }

    /// A thread as this version's T3 Code writes it, with its native session.
    pub fn thread(&self, row: &ThreadRow<'_>) {
        if self.v2() {
            self.v2_thread(row);
        } else {
            self.v1_thread(row);
            if let Some(native) = row.native {
                self.cursor(row.id, row.driver, native);
            }
        }
    }

    fn stamp(value: bool) -> Value {
        if value { t("2026-11-03T00:00:00.000Z") } else { Value::Null }
    }

    fn v1_thread(&self, row: &ThreadRow<'_>) {
        let text = |value: Option<&str>| value.map_or(Value::Null, |value| t(value));
        self.insert("projection_threads", &[
            ("thread_id", t(row.id)),
            ("project_id", t(row.project)),
            ("title", t(row.title)),
            ("model", t("claude-sonnet-4-5")),
            ("model_selection_json", t(json!({"provider": row.driver, "model": "claude-sonnet-4-5"}).to_string())),
            ("branch", text(row.branch)),
            ("worktree_path", row.worktree.map_or(Value::Null, |path| t(path.to_string_lossy().into_owned()))),
            ("created_at", t("2026-11-01T00:00:00.000Z")),
            ("updated_at", t(row.updated_at)),
            ("archived_at", Self::stamp(row.archived)),
            ("deleted_at", Self::stamp(row.deleted)),
            ("pinned_at", Self::stamp(row.pinned)),
        ]);
    }

    /// A v1 resume cursor in `provider_session_runtime`, as each driver
    /// writes it.
    pub fn cursor(&self, thread: &str, driver: &str, native: &str) {
        let cursor = if super::store::harness_of(driver) == Some("claude") {
            json!({"threadId": thread, "resume": native, "resumeSessionAt": "m-1", "turnCount": 1})
        } else {
            json!({"threadId": native})
        };
        self.insert("provider_session_runtime", &[
            ("thread_id", t(thread)),
            ("provider_name", t(driver)),
            ("adapter_key", t(driver)),
            ("status", t("stopped")),
            ("last_seen_at", t("2026-11-01T00:00:00.000Z")),
            ("resume_cursor_json", t(cursor.to_string())),
            ("runtime_payload_json", t(json!({"cwd": "/somewhere", "secret": CANARY}).to_string())),
        ]);
    }

    fn v2_thread(&self, row: &ThreadRow<'_>) {
        let provider_thread = row.native.map(|native| format!("provider-thread:provider:{}:native-thread:{native}", row.driver));
        let payload = json!({
            "id": row.id,
            "projectId": row.project,
            "title": row.title,
            "providerInstanceId": row.driver,
            "modelSelection": {"instanceId": row.driver, "model": "claude-haiku-4-5"},
            "branch": row.branch,
            "worktreePath": row.worktree.map(|path| path.to_string_lossy().into_owned()),
            "pullRequests": [],
            "activeProviderThreadId": provider_thread,
            "lineage": {
                "parentThreadId": row.parent,
                "relationshipToParent": row.parent.map(|_| "subagent"),
                "rootThreadId": row.parent.unwrap_or(row.id),
            },
            "pinnedAt": if row.pinned { Some("2026-11-03T00:00:00.000Z") } else { None },
            "historyOrigin": "native",
            "createdAt": "2026-11-01T00:00:00.000Z",
            "updatedAt": row.updated_at,
            "deletedAt": if row.deleted { Some("2026-11-03T00:00:00.000Z") } else { None },
        });
        self.insert("orchestration_v2_projection_threads", &[
            ("thread_id", t(row.id)),
            ("project_id", t(row.project)),
            ("title", t(row.title)),
            ("default_provider", t(row.driver)),
            ("runtime_mode", t("full-access")),
            ("interaction_mode", t("default")),
            ("active_provider_thread_id", provider_thread.clone().map_or(Value::Null, t)),
            ("created_at", t("2026-11-01T00:00:00.000Z")),
            ("updated_at", t(row.updated_at)),
            ("archived_at", Self::stamp(row.archived)),
            ("deleted_at", Self::stamp(row.deleted)),
            ("payload_json", t(payload.to_string())),
            ("provider_instance_id", t(row.driver)),
        ]);
        if let (Some(id), Some(native)) = (provider_thread, row.native) {
            self.provider_thread(&id, row.id, row.driver, native, row.updated_at);
        }
    }

    /// A v2 provider thread pointing at a native session.
    pub fn provider_thread(&self, id: &str, thread: &str, driver: &str, native: &str, updated_at: &str) {
        let payload = json!({
            "id": id,
            "driver": driver,
            "appThreadId": thread,
            "nativeThreadRef": {"driver": driver, "nativeId": native, "strength": "strong"},
            "status": "idle",
        });
        self.insert("orchestration_v2_projection_provider_threads", &[
            ("provider_thread_id", t(id)),
            ("thread_id", t(thread)),
            ("provider", t(driver)),
            ("driver", t(driver)),
            ("provider_session_id", t(format!("provider-session:provider-instance:{driver}:shared"))),
            ("status", t("idle")),
            ("updated_at", t(updated_at)),
            ("payload_json", t(payload.to_string())),
        ]);
    }

    /// A message in this version's message table.
    pub fn message(&self, thread: &str, id: &str, role: &str, text: &str, at: &str, streaming: bool) {
        if self.v2() {
            self.insert("orchestration_v2_projection_messages", &[
                ("message_id", t(id)),
                ("thread_id", t(thread)),
                ("role", t(role)),
                ("streaming", i64::from(streaming).into()),
                ("created_at", t(at)),
                ("updated_at", t(at)),
                ("payload_json", t(json!({"id": id, "role": role, "text": text}).to_string())),
            ]);
        } else {
            self.legacy_message(thread, id, role, text, at, streaming);
        }
    }

    /// A message in the v1 table (in a v2 file: left behind by the upgrade).
    pub fn legacy_message(&self, thread: &str, id: &str, role: &str, text: &str, at: &str, streaming: bool) {
        self.insert("projection_thread_messages", &[
            ("message_id", t(id)),
            ("thread_id", t(thread)),
            ("role", t(role)),
            ("text", t(text)),
            ("is_streaming", i64::from(streaming).into()),
            ("created_at", t(at)),
            ("updated_at", t(at)),
        ]);
    }

    /// A thread T3 Code upgraded from v1: a v2 shell without a provider
    /// thread, its cursor left in `provider_session_runtime`, and its
    /// transcript copied over or not.
    pub fn upgraded_thread(&self, row: &ThreadRow<'_>, transcribed: bool) {
        self.v2_thread(&ThreadRow { native: None, ..row.clone() });
        if let Some(native) = row.native {
            self.cursor(row.id, row.driver, native);
        }
        self.conn
            .execute(
                "UPDATE orchestration_v2_projection_threads SET payload_json=json_set(payload_json,'$.historyOrigin','v1_import') WHERE thread_id=?1",
                [row.id],
            )
            .unwrap();
        self.insert("orchestration_v2_legacy_imports", &[
            ("thread_id", t(row.id)),
            ("source_updated_at", t(row.updated_at)),
            ("shell_imported_at", t("2026-11-20T00:00:00.000Z")),
            ("transcript_imported_at", if transcribed { t("2026-11-20T00:00:00.000Z") } else { Value::Null }),
        ]);
    }
}

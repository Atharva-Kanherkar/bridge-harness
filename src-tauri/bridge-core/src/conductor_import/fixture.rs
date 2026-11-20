//! A fixture `conductor.db` for tests, built from one of the real schemas
//! rebuilt from Conductor's rollbacks (`testing/fixtures/import/conductor/`).
//! Rows go into the columns that version has, as Conductor writes them. Its
//! `env_vars` and `settings` tables hold canaries no output may contain.

use rusqlite::types::Value;
use rusqlite::{params_from_iter, Connection};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const CANARY: &str = "CANARY-conductor-secret";

/// A text value.
fn t(value: impl Into<String>) -> Value {
    Value::Text(value.into())
}
/// The real schema versions there are fixtures for, oldest first.
pub const VERSIONS: &[i64] = &[80, 99, 119, 149, 178, 181];

pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testing/fixtures/import/conductor")
}

pub fn schema(version: i64) -> String {
    std::fs::read_to_string(fixtures_dir().join(format!("schema-{version}.sql"))).unwrap()
}

pub fn migrations(version: i64) -> Vec<(i64, String)> {
    std::fs::read_to_string(fixtures_dir().join(format!("migrations-{version}.txt")))
        .unwrap()
        .lines()
        .filter_map(|line| line.split_once('|'))
        .map(|(number, description)| (number.parse().unwrap(), description.to_owned()))
        .collect()
}

pub struct Fixture {
    pub path: PathBuf,
    pub conn: Connection,
}

/// A workspace row as a test describes it; `None` leaves a column empty.
#[derive(Debug, Clone, Default)]
pub struct Row<'a> {
    pub key: &'a str,
    pub repo: &'a str,
    pub directory: &'a str,
    pub name: Option<&'a str>,
    pub branch: Option<&'a str>,
    pub state: &'a str,
    pub path: Option<&'a Path>,
}

impl Fixture {
    /// `<dir>/conductor.db` at a real schema version.
    pub fn new(dir: &Path, version: i64) -> Self {
        Self::at(&dir.join("conductor.db"), version)
    }

    pub fn at(path: &Path, version: i64) -> Self {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let conn = Connection::open(path).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        conn.execute_batch(&schema(version)).unwrap();
        for (number, description) in migrations(version) {
            conn.execute(
                "INSERT INTO _sqlx_migrations(version,description,installed_on,success,checksum,execution_time) VALUES(?1,?2,'2026-11-20',1,x'00',1)",
                rusqlite::params![number, description],
            )
            .unwrap();
        }
        let fixture = Self { path: path.to_path_buf(), conn };
        fixture.insert("settings", &[("key", t("secret_key")), ("value", t(CANARY))]);
        fixture.insert("env_vars", &[("key", t("API_TOKEN")), ("value", t(CANARY)), ("scope", t("user"))]);
        fixture
    }

    fn columns(&self, table: &str) -> HashSet<String> {
        let mut statement = self.conn.prepare(&format!("SELECT name FROM pragma_table_info('{table}')")).unwrap();
        statement.query_map([], |row| row.get(0)).unwrap().map(Result::unwrap).collect()
    }

    pub fn has(&self, table: &str, column: &str) -> bool {
        self.columns(table).contains(column)
    }

    /// Insert into the columns this version has; others are dropped.
    pub fn insert(&self, table: &str, values: &[(&str, Value)]) {
        self.try_insert(table, values).unwrap();
    }

    /// [`Self::insert`], returning what SQLite says (a repeated key, say).
    pub fn try_insert(&self, table: &str, values: &[(&str, Value)]) -> rusqlite::Result<()> {
        let present = self.columns(table);
        if present.is_empty() {
            return Ok(());
        }
        let kept: Vec<&(&str, Value)> = values.iter().filter(|(column, _)| present.contains(*column)).collect();
        let sql = format!(
            "INSERT INTO {table}({}) VALUES({})",
            kept.iter().map(|(column, _)| format!("\"{column}\"")).collect::<Vec<_>>().join(","),
            vec!["?"; kept.len()].join(",")
        );
        self.conn.execute(&sql, params_from_iter(kept.iter().map(|(_, value)| value.clone())))?;
        Ok(())
    }

    pub fn repo(&self, id: &str, name: &str, root: &Path, storage_version: i64) {
        self.insert("repos", &[
            ("id", t(id)),
            ("name", t(name)),
            ("root_path", t(root.to_string_lossy().into_owned())),
            ("storage_version", storage_version.into()),
        ]);
        // From migration 179, a workspace belongs to a project of repositories.
        self.insert("projects", &[("id", t(format!("project-{id}"))), ("name", t(name))]);
        self.insert("project_repositories", &[
            ("id", t(format!("pr-{id}"))),
            ("project_id", t(format!("project-{id}"))),
            ("repository_id", t(id)),
            ("position", 0.into()),
        ]);
    }

    pub fn scripts(&self, repo: &str, setup: &str, run: &str, archive: &str) {
        self.conn
            .execute(
                "UPDATE repos SET setup_script=?2, run_script=?3, archive_script=?4, run_script_mode='nonconcurrent' WHERE id=?1",
                rusqlite::params![repo, setup, run, archive],
            )
            .unwrap();
    }

    pub fn workspace(&self, row: &Row<'_>) {
        let text = |value: Option<&str>| value.map_or(Value::Null, |value| Value::Text(value.to_owned()));
        let key_column = if self.has("workspaces", "local_id") { "local_id" } else { "id" };
        self.insert("workspaces", &[
            (key_column, t(row.key)),
            ("repository_id", t(row.repo)),
            ("project_id", t(format!("project-{}", row.repo))),
            ("directory_name", t(row.directory)),
            ("workspace_name", text(row.name)),
            ("branch", text(row.branch)),
            ("state", t(row.state)),
            ("DEPRECATED_archived", i64::from(row.state == "archived").into()),
            ("workspace_path", text(row.path.map(|path| path.to_str().unwrap()))),
        ]);
    }

    pub fn checkout(&self, workspace: &str, repo: &str, path: Option<&Path>, position: i64) {
        // A repeated checkout is refused by its key; the first one stands.
        self.try_insert("workspace_checkouts", &[
            ("id", t(format!("co-{workspace}-{repo}"))),
            ("workspace_id", t(workspace)),
            ("repository_id", t(repo)),
            ("path", path.map_or(Value::Null, |path| Value::Text(path.to_string_lossy().into_owned()))),
            ("position", position.into()),
        ]).unwrap_or_default();
    }

    pub fn session(&self, id: &str, workspace: &str, agent: &str) {
        self.insert("sessions", &[("id", t(id)), ("workspace_id", t(workspace)), ("agent_type", t(agent)), ("title", t(CANARY))]);
    }
}

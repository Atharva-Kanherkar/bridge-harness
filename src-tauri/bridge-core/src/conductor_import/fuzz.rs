//! Fuzzing Conductor's import (property-based, with `proptest`).
//!
//! - **Plan**: random candidates and git answers. Each candidate gets one
//!   outcome, nothing is adopted twice, nothing archived is adopted, and a
//!   second run adopts nothing.
//! - **Store**: random rows, with random values and types, on every real
//!   schema version. It reads, never panics, queries only allowed tables,
//!   never yields a secret, and every path is absolute and follows the rule.
//! - **Schema mutation**: the real schemas with columns renamed, tables
//!   dropped, versions shifted and rows of the wrong type. It fails closed
//!   or reads correctly; it never panics.
//! - **Filesystem**: small layouts of real repositories and worktrees,
//!   adopted end to end. Exactly the valid, unarchived worktrees on a branch
//!   come over, and no file on disk changes.

use super::fixture::{Fixture, CANARY, VERSIONS};
use super::*;
use crate::import_worktrees::{plan, Outcome, Probe};
use proptest::prelude::*;
use rusqlite::types::Value;
use std::collections::BTreeMap;

fn tricky_text() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(String::new()),
        Just(".".to_owned()),
        Just("..".to_owned()),
        Just("a/b".to_owned()),
        Just("tokyo".to_owned()),
        Just("'; DROP TABLE workspaces; --".to_owned()),
        Just("são paulo".to_owned()),
        Just("名前".to_owned()),
        "[a-z]{1,8}",
        ".{0,20}",
    ]
}

/// A value of any type Conductor's untyped columns might hold.
fn any_value() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(Value::Null),
        any::<i64>().prop_map(Value::Integer),
        (-2i64..6).prop_map(Value::Integer),
        tricky_text().prop_map(Value::Text),
        Just(Value::Real(1.5)),
        Just(Value::Blob(vec![0, 159, 146, 150])),
    ]
}

fn state() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("active".to_owned()),
        Just("archived".to_owned()),
        Just("deleted".to_owned()),
        Just("initializing".to_owned()),
        Just("setting_up".to_owned()),
        tricky_text(),
    ]
}

fn probe() -> impl Strategy<Value = Probe> {
    prop_oneof![
        Just(Probe::Missing),
        Just(Probe::NotGit),
        Just(Probe::MainCheckout),
        Just(Probe::Unregistered),
        Just(Probe::Detached { root: PathBuf::from("/r") }),
        Just(Probe::OnBranch { root: PathBuf::from("/r"), branch: "b".into() }),
    ]
}

fn plan_candidate(index: usize, archived: bool) -> Candidate {
    Candidate {
        tool: PROVIDER,
        path: PathBuf::from(format!("/w/{index}")),
        name: index.to_string(),
        title: None,
        archived,
        source_ref: format!("conductor:{index}"),
        scripts: None,
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2000, ..ProptestConfig::default() })]

    #[test]
    fn plan_gives_every_candidate_one_outcome_and_adopts_nothing_twice(
        items in proptest::collection::vec((0usize..6, any::<bool>(), probe()), 0..30),
        known in proptest::collection::vec(any::<bool>(), 6),
    ) {
        let candidates: Vec<(Candidate, Probe)> = items.iter().map(|(index, archived, probe)| (plan_candidate(*index, *archived), probe.clone())).collect();
        let in_bridge: HashSet<PathBuf> = known.iter().enumerate().filter(|(_, known)| **known).map(|(index, _)| PathBuf::from(format!("/w/{index}"))).collect();

        let outcomes = plan(&candidates, &in_bridge);

        prop_assert_eq!(outcomes.len(), candidates.len());
        prop_assert_eq!(&outcomes, &plan(&candidates, &in_bridge), "the plan is deterministic");
        let mut adopted = HashSet::new();
        for ((candidate, probe), outcome) in candidates.iter().zip(&outcomes) {
            if let Outcome::Adopt { .. } = outcome {
                prop_assert!(!candidate.archived, "nothing archived is adopted");
                prop_assert!(matches!(probe, Probe::OnBranch { .. }), "only a worktree on a branch is adopted");
                prop_assert!(!in_bridge.contains(&candidate.path), "a folder Bridge has is not adopted again");
                prop_assert!(adopted.insert(candidate.path.clone()), "no folder is adopted twice");
            }
        }
        let after: HashSet<PathBuf> = in_bridge.union(&adopted).cloned().collect();
        prop_assert!(plan(&candidates, &after).iter().all(|outcome| !matches!(outcome, Outcome::Adopt { .. })), "a second run adopts nothing");
    }
}

/// One random repository row.
#[derive(Debug, Clone)]
struct RepoSpec {
    name: String,
    root: Value,
    storage: Value,
    scripts: Value,
}

/// One random workspace row.
#[derive(Debug, Clone)]
struct WorkspaceSpec {
    key: u8,
    repo: u8,
    directory: String,
    name: Value,
    branch: Value,
    state: String,
    path: Value,
    flag: Value,
}

fn repo_spec() -> impl Strategy<Value = RepoSpec> {
    (
        tricky_text(),
        prop_oneof![
            Just(Value::Text("/code/app".into())),
            Just(Value::Text("/code/my app".into())),
            Just(Value::Text("relative/root".into())),
            any_value()
        ],
        any_value(),
        any_value(),
    )
        .prop_map(|(name, root, storage, scripts)| RepoSpec { name, root, storage, scripts })
}

fn workspace_spec() -> impl Strategy<Value = WorkspaceSpec> {
    (0u8..8, 0u8..4, tricky_text(), any_value(), any_value(), state(), prop_oneof![Just(Value::Null), Just(Value::Text("/elsewhere/ws".into())), any_value()], any_value())
        .prop_map(|(key, repo, directory, name, branch, state, path, flag)| WorkspaceSpec { key, repo, directory, name, branch, state, path, flag })
}

fn fill(fixture: &Fixture, repos: &[RepoSpec], workspaces: &[WorkspaceSpec]) {
    for (index, repo) in repos.iter().enumerate() {
        let id = format!("r{index}");
        fixture.insert("repos", &[
            ("id", Value::Text(id.clone())),
            ("name", Value::Text(repo.name.clone())),
            ("root_path", repo.root.clone()),
            ("storage_version", repo.storage.clone()),
            ("setup_script", repo.scripts.clone()),
        ]);
        fixture.insert("projects", &[("id", Value::Text(format!("project-{id}")))]);
        fixture.insert("project_repositories", &[
            ("id", Value::Text(format!("pr-{id}"))),
            ("project_id", Value::Text(format!("project-{id}"))),
            ("repository_id", Value::Text(id)),
        ]);
    }
    let key_column = if fixture.has("workspaces", "local_id") { "local_id" } else { "id" };
    for (index, workspace) in workspaces.iter().enumerate() {
        // Keys repeat on purpose; the table refuses a repeated primary key,
        // which is fine: what got in is what Conductor would hold.
        let key = format!("w{}", workspace.key);
        let repo = format!("r{}", workspace.repo);
        let values = [
            (key_column, Value::Text(key.clone())),
            ("repository_id", Value::Text(repo.clone())),
            ("project_id", Value::Text(format!("project-{repo}"))),
            ("directory_name", Value::Text(workspace.directory.clone())),
            ("workspace_name", workspace.name.clone()),
            ("branch", workspace.branch.clone()),
            ("state", Value::Text(workspace.state.clone())),
            ("workspace_path", workspace.path.clone()),
            ("DEPRECATED_archived", workspace.flag.clone()),
        ];
        fixture.try_insert("workspaces", &values).unwrap_or_default();
        if index % 3 == 0 {
            fixture.checkout(&key, &repo, None, index as i64);
        }
        fixture.session(&format!("s{index}"), &key, "claude");
    }
}

fn assert_safe(discovery: &Discovery, paths: &Paths) -> Result<(), TestCaseError> {
    // Canaries sit in env_vars, settings and chat titles, which are never read.
    let shown = format!("{discovery:?}");
    prop_assert!(!shown.contains(CANARY), "a secret came out");
    for candidate in &discovery.candidates {
        prop_assert!(candidate.path.is_absolute(), "{:?}", candidate.path);
        prop_assert!(candidate.path != paths.workspaces_root, "the workspaces folder itself is not a workspace");
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 300, ..ProptestConfig::default() })]

    #[test]
    fn random_rows_on_every_real_schema_read_safely(
        version in proptest::sample::select(VERSIONS.to_vec()),
        repos in proptest::collection::vec(repo_spec(), 0..4),
        workspaces in proptest::collection::vec(workspace_spec(), 0..16),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::for_home(dir.path());
        let fixture = Fixture::at(&paths.store, version);
        fill(&fixture, &repos, &workspaces);
        STATEMENTS.with(|statements| statements.borrow_mut().clear());

        let read = read(&paths.store);
        prop_assert!(read.is_ok(), "an intact real schema reads: {:?}", read);
        let store = read.unwrap();
        let discovery = discover(&paths);

        prop_assert_eq!(store.version, version);
        prop_assert_eq!(&discovery, &discover(&paths), "reading is deterministic");
        assert_safe(&discovery, &paths)?;
        for statement in STATEMENTS.with(|statements| statements.borrow().clone()) {
            let lower = statement.to_lowercase();
            prop_assert!(lower.starts_with("select "));
            prop_assert!(DENIED_TABLES.iter().all(|denied| !lower.contains(&format!("from {denied}")) && !lower.contains(&format!("'{denied}'"))), "{}", statement);
        }
        for workspace in &store.workspaces {
            let state_archived = workspace.archived;
            if let Some(candidate) = discovery.candidates.iter().find(|candidate| candidate.source_ref.starts_with(&format!("conductor:{}", workspace.keys[0]))) {
                prop_assert_eq!(candidate.archived, state_archived);
                if workspace.path.is_none() && workspace.checkouts.iter().all(|checkout| checkout.path.is_none()) {
                    let directory = workspace.directory.trim();
                    prop_assert!(!directory.is_empty() && directory != "." && directory != ".." && !directory.contains('/'));
                    prop_assert_eq!(candidate.path.file_name().map(|name| name.to_string_lossy().into_owned()), Some(directory.to_owned()));
                }
            }
        }
    }
}

/// One change to a real schema, as a newer, older or damaged Conductor
/// might leave it.
#[derive(Debug, Clone)]
enum Mutation {
    DropTable(&'static str),
    RenameColumn(&'static str, &'static str),
    AddColumn(&'static str),
    Version(i64),
    ClearMigrations,
    FailMigrations,
    WrongTypes,
}

const REQUIRED: &[(&str, &str)] = &[
    ("workspaces", "directory_name"),
    ("workspaces", "branch"),
    ("workspaces", "state"),
    ("repos", "id"),
    ("repos", "name"),
    ("repos", "root_path"),
];

fn mutation() -> impl Strategy<Value = Mutation> {
    prop_oneof![
        proptest::sample::select(vec!["workspaces", "repos", "_sqlx_migrations", "sessions", "project_repositories", "workspace_checkouts", "projects"]).prop_map(Mutation::DropTable),
        proptest::sample::select(vec![
            ("workspaces", "directory_name"), ("workspaces", "branch"), ("workspaces", "state"), ("workspaces", "workspace_name"),
            ("workspaces", "workspace_path"), ("repos", "root_path"), ("repos", "name"), ("repos", "storage_version"), ("repos", "setup_script"),
        ]).prop_map(|(table, column)| Mutation::RenameColumn(table, column)),
        proptest::sample::select(vec!["workspaces", "repos", "sessions"]).prop_map(Mutation::AddColumn),
        prop_oneof![Just(i64::MIN), Just(-1i64), Just(0i64), 1i64..400, Just(i64::MAX)].prop_map(Mutation::Version),
        Just(Mutation::ClearMigrations),
        Just(Mutation::FailMigrations),
        Just(Mutation::WrongTypes),
    ]
}

/// Apply a mutation; `true` when it took effect.
fn mutate(conn: &Connection, mutation: &Mutation) -> bool {
    let sql = match mutation {
        Mutation::DropTable(table) => format!("PRAGMA foreign_keys=OFF; DROP TABLE {table};"),
        Mutation::RenameColumn(table, column) => format!("ALTER TABLE {table} RENAME COLUMN \"{column}\" TO \"{column}_renamed\";"),
        Mutation::AddColumn(table) => format!("ALTER TABLE {table} ADD COLUMN surprise BLOB;"),
        Mutation::Version(version) => format!("INSERT INTO _sqlx_migrations(version,description,installed_on,success,checksum,execution_time) VALUES({version},'x',1,1,x'00',1);"),
        Mutation::ClearMigrations => "DELETE FROM _sqlx_migrations;".into(),
        Mutation::FailMigrations => "UPDATE _sqlx_migrations SET success=0;".into(),
        Mutation::WrongTypes => "UPDATE workspaces SET directory_name=x'00ff', branch=3.5, state=42; UPDATE repos SET root_path=7, storage_version='three';".into(),
    };
    conn.execute_batch(&sql).is_ok()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 400, ..ProptestConfig::default() })]

    #[test]
    fn a_mutated_schema_fails_closed_or_reads_correctly(
        version in proptest::sample::select(VERSIONS.to_vec()),
        mutations in proptest::collection::vec(mutation(), 1..4),
        workspaces in proptest::collection::vec(workspace_spec(), 0..6),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::for_home(dir.path());
        let fixture = Fixture::at(&paths.store, version);
        fill(&fixture, &[RepoSpec { name: "app".into(), root: Value::Text("/code/app".into()), storage: Value::Integer(3), scripts: Value::Null }], &workspaces);
        let applied: Vec<&Mutation> = mutations.iter().filter(|mutation| mutate(&fixture.conn, mutation)).collect();

        let read = read(&paths.store);
        let discovery = discover(&paths);

        assert_safe(&discovery, &paths)?;
        // An independent look at what the mutations left, to say what the
        // reader must do: read when everything it needs is there at a known
        // version, else fail closed with a reason and folders only.
        let expect_read = readable(&fixture.conn);
        prop_assert_eq!(read.is_ok(), expect_read, "{:?} gave {:?}", applied, read);
        if !expect_read {
            prop_assert!(discovery.note.is_some(), "a database Bridge cannot use says why");
            prop_assert!(discovery.candidates.iter().all(|candidate| candidate.title.is_none()), "and brings folders only");
        }
        if let Ok(store) = &read {
            prop_assert!((OLDEST_MIGRATION..=NEWEST_MIGRATION).contains(&store.version));
        }
    }
}

/// Whether a database has everything the reader needs at a version it knows,
/// asked of SQLite directly rather than through the reader.
fn readable(conn: &Connection) -> bool {
    let column = |table: &str, column: &str| -> bool {
        conn.query_row(&format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name=?1"), [column], |row| row.get::<_, i64>(0)).unwrap_or(0) > 0
    };
    let version: Option<i64> = conn
        .query_row("SELECT MAX(version) FROM _sqlx_migrations WHERE success IS NULL OR success != 0", [], |row| row.get(0))
        .unwrap_or(None);
    REQUIRED.iter().all(|(table, name)| column(table, name))
        && (column("workspaces", "local_id") || column("workspaces", "id"))
        && (column("workspaces", "repository_id") || column("workspaces", "project_id"))
        && version.is_some_and(|version| (OLDEST_MIGRATION..=NEWEST_MIGRATION).contains(&version))
}

/// What a folder in a filesystem layout is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Folder {
    OnBranch,
    Dirty,
    Detached,
    Deleted,
    Plain,
}

fn folder() -> impl Strategy<Value = Folder> {
    prop_oneof![Just(Folder::OnBranch), Just(Folder::Dirty), Just(Folder::Detached), Just(Folder::Deleted), Just(Folder::Plain)]
}

/// Every file under `root`, with its size and contents.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file() {
                out.insert(path.strip_prefix(root).unwrap().to_path_buf(), std::fs::read(&path).unwrap());
            }
        }
    }
    out
}

proptest! {
    // Real git: few cases, so the machine is not loaded.
    #![proptest_config(ProptestConfig { cases: 16, ..ProptestConfig::default() })]

    #[test]
    fn real_layouts_adopt_exactly_the_valid_workspaces_and_change_no_file(
        layout in proptest::collection::vec(proptest::collection::vec((folder(), any::<bool>(), any::<bool>()), 0..4), 1..3),
    ) {
        use crate::import_worktrees::tests::{git, repo, worktree};
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let paths = Paths::for_home(&home);
        let fixture = Fixture::at(&paths.store, 181);
        let mut expected = HashSet::new();
        for (r, folders) in layout.iter().enumerate() {
            let name = format!("repo{r}");
            let main = repo(&home.join("code"), &name);
            fixture.repo(&format!("r{r}"), &name, &main, 3);
            for (w, (kind, in_db, archived)) in folders.iter().enumerate() {
                let directory = format!("ws{w}");
                let path = home.join("conductor/workspaces").join(&name).join(&directory);
                match kind {
                    Folder::OnBranch | Folder::Dirty | Folder::Deleted => {
                        worktree(&main, &path, &format!("{name}-{directory}"));
                    }
                    Folder::Detached => git(&main, &["worktree", "add", "-q", "--detach", &path.to_string_lossy()]),
                    Folder::Plain => std::fs::create_dir_all(&path).unwrap(),
                }
                if *kind == Folder::Dirty {
                    std::fs::write(path.join("README.md"), "uncommitted\n").unwrap();
                }
                if *kind == Folder::Deleted {
                    std::fs::remove_dir_all(&path).unwrap();
                }
                if *in_db {
                    fixture.workspace(&super::fixture::Row { key: &format!("k{r}-{w}"), repo: &format!("r{r}"), directory: &directory, name: None, branch: Some("b"), state: if *archived { "archived" } else { "active" }, path: None });
                }
                let archived_in_db = *in_db && *archived;
                if matches!(kind, Folder::OnBranch | Folder::Dirty) && !archived_in_db {
                    expected.insert(path.clone());
                }
            }
        }
        fixture.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
        let before = snapshot(&home);
        let scratch = tempfile::tempdir().unwrap();
        let core = crate::runtime::BridgeCore::for_tests(scratch.path());

        let report = crate::import_worktrees::adopt(&core.db, discover(&paths).candidates).unwrap();
        let again = crate::import_worktrees::adopt(&core.db, discover(&paths).candidates).unwrap();

        let adopted: HashSet<PathBuf> = report.adopted.iter().map(|(_, path)| path.clone()).collect();
        prop_assert_eq!(&adopted, &expected);
        prop_assert!(again.adopted.is_empty());
        prop_assert_eq!(again.already_in_bridge, expected.len());
        prop_assert!(snapshot(&home) == before, "no file on disk changed");
    }
}

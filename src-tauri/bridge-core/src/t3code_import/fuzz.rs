//! Fuzzing T3 Code's import (property-based, with `proptest`).
//!
//! - **Store**: random rows, with random values and types, on every real
//!   schema head. It reads, never panics, queries only allowed tables, never
//!   yields a secret, and reads back what was written.
//! - **Schema mutation**: the real schemas with columns renamed, tables
//!   dropped, versions shifted and payloads of the wrong type. It fails
//!   closed or reads correctly against an independent oracle; it never
//!   panics.
//! - **Plan**: random thread sets and random sets of transcripts found. Every
//!   live thread's chat comes over exactly one way (or with its Claude
//!   parent), deleted ones never, and each worktree is a candidate once.

use super::fixture::{Fixture, ThreadRow, CANARY, VERSIONS};
use super::store::{self, Era, NativeSession, Project, Thread, DENIED_TABLES, NEWEST_MIGRATION, OLDEST_MIGRATION};
use super::*;
use crate::import_sqlite::STATEMENTS;
use proptest::prelude::*;
use rusqlite::types::Value as Sql;

/// A random value of any SQLite type, garbage included.
fn any_value() -> impl Strategy<Value = Sql> {
    prop_oneof![
        Just(Sql::Null),
        any::<i64>().prop_map(Sql::Integer),
        any::<f64>().prop_map(Sql::Real),
        "[ -~]{0,24}".prop_map(Sql::Text),
        Just(Sql::Text("{\"nativeThreadRef\":{\"driver\":\"claudeAgent\",\"nativeId\":\"x\"}}".into())),
        Just(Sql::Text("not json".into())),
        Just(Sql::Text("[1,2,3]".into())),
        proptest::collection::vec(any::<u8>(), 0..8).prop_map(Sql::Blob),
    ]
}

#[derive(Debug, Clone)]
struct Spec {
    id: String,
    project: usize,
    title: String,
    driver: &'static str,
    native: Option<String>,
    worktree: Option<String>,
    archived: bool,
    deleted: bool,
    pinned: bool,
    parent: Option<usize>,
    messages: Vec<(bool, String, bool)>,
}

fn spec() -> impl Strategy<Value = Spec> {
    (
        "[a-z0-9:-]{1,12}",
        0usize..3,
        "[ -~]{0,20}",
        proptest::sample::select(vec!["claudeAgent", "codex", "opencode", "cursor", "grok"]),
        proptest::option::of("[a-f0-9-]{1,36}"),
        proptest::option::of(proptest::sample::select(vec!["/w/shop/a", "/w/shop/b", "/w/api/c", "relative/d"])),
        (any::<bool>(), any::<bool>(), any::<bool>()),
        proptest::option::of(0usize..6),
        proptest::collection::vec((any::<bool>(), "[ -~]{0,16}", any::<bool>()), 0..4),
    )
        .prop_map(|(id, project, title, driver, native, worktree, (archived, deleted, pinned), parent, messages)| Spec {
            id,
            project,
            title,
            driver,
            native,
            worktree: worktree.map(str::to_owned),
            archived,
            deleted,
            pinned,
            parent,
            messages,
        })
}

fn fill(fixture: &Fixture, specs: &[Spec]) {
    for project in 0..3 {
        fixture.project(&format!("p{project}"), Path::new(&format!("/code/repo{project}")), &format!("Repo {project}"));
    }
    fixture.delete_project("p2");
    let ids: Vec<String> = specs.iter().map(|spec| spec.id.clone()).collect();
    // One provider thread belongs to one app thread in T3 Code (its id is
    // unique), so a later thread on the same driver and native id gets none.
    let mut natives = std::collections::HashSet::new();
    for (index, spec) in specs.iter().enumerate() {
        if ids[..index].contains(&spec.id) {
            continue;
        }
        let native = spec.native.as_deref().filter(|native| natives.insert((spec.driver, *native)));
        let parent = spec.parent.and_then(|parent| ids.get(parent)).filter(|parent| **parent != spec.id).map(String::as_str);
        let worktree = spec.worktree.as_deref().map(Path::new);
        let project = format!("p{}", spec.project);
        let row = ThreadRow {
            id: &spec.id,
            project: &project,
            title: &spec.title,
            worktree,
            archived: spec.archived,
            deleted: spec.deleted,
            pinned: spec.pinned,
            parent: if fixture.v2() { parent } else { None },
            driver: spec.driver,
            native,
            updated_at: "2026-11-05T00:00:00.000Z",
            ..ThreadRow::default()
        };
        fixture.thread(&row);
        for (number, (user, text, streaming)) in spec.messages.iter().enumerate() {
            fixture.message(&spec.id, &format!("{}-{number}", spec.id), if *user { "user" } else { "assistant" }, text, &format!("2026-11-05T00:00:0{number}.000Z"), *streaming);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 300, ..ProptestConfig::default() })]

    #[test]
    fn random_rows_on_every_real_schema_read_safely(
        version in proptest::sample::select(VERSIONS.to_vec()),
        specs in proptest::collection::vec(spec(), 0..12),
        garbage in proptest::collection::vec((0usize..3, any_value()), 0..4),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let fixture = Fixture::in_base(dir.path(), version);
        fill(&fixture, &specs);
        // Garbage in the columns Bridge reads loosely.
        for (target, value) in &garbage {
            let sql = match target {
                0 => "UPDATE projection_projects SET scripts_json=?1",
                1 if fixture.v2() => "UPDATE orchestration_v2_projection_threads SET payload_json=?1 WHERE rowid=1",
                1 => "UPDATE projection_threads SET title=?1 WHERE rowid=1",
                _ => "UPDATE provider_session_runtime SET resume_cursor_json=?1",
            };
            fixture.conn.execute(sql, [value]).unwrap_or_default();
        }
        STATEMENTS.with(|statements| statements.borrow_mut().clear());

        let read = store::read(&fixture.conn);
        prop_assert!(read.is_ok(), "an intact real schema reads: {:?}", read);
        let store = read.unwrap();
        prop_assert_eq!(store.version, version);
        prop_assert_eq!(store.era, if version >= 55 { Era::V2 } else { Era::V1 });
        let mut shown = format!("{store:?}");
        for thread in &store.threads {
            let messages = store::messages(&fixture.conn, store.era, thread);
            prop_assert!(messages.is_ok(), "{:?}", messages);
            let messages = messages.unwrap();
            for message in &messages {
                prop_assert!(!message.text.trim().is_empty());
            }
            let chat = copy_chat(&store, thread, &messages, "ref");
            prop_assert_eq!(chat.entries.len(), messages.len());
            if let Some(cwd) = &chat.cwd {
                prop_assert!(Path::new(cwd).is_absolute(), "{}", cwd);
            }
            shown.push_str(&serde_json::to_string(&chat).unwrap());
        }
        prop_assert!(!shown.contains(CANARY), "a secret came out");
        for statement in STATEMENTS.with(|statements| statements.borrow().clone()) {
            let lower = statement.to_lowercase();
            prop_assert!(lower.starts_with("select "));
            prop_assert!(DENIED_TABLES.iter().all(|denied| !lower.contains(&format!("from {denied}")) && !lower.contains(&format!("'{denied}'"))), "{}", statement);
            prop_assert!(!lower.contains("runtime_payload_json"));
        }
        for candidate in worktree_candidates(&store) {
            prop_assert!(candidate.path.is_absolute());
        }
        prop_assert_eq!(&store, &store::read(&fixture.conn).unwrap(), "reading is deterministic");
    }
}

/// One change to a real schema, as a newer, older or damaged T3 Code might
/// leave it.
#[derive(Debug, Clone)]
enum Mutation {
    DropTable(&'static str),
    RenameColumn(&'static str, &'static str),
    AddColumn(&'static str),
    Version(i64),
    ClearMigrations,
    WrongTypes,
}

/// What [`store::read`] needs, per era.
const REQUIRED_V1: &[(&str, &str)] = &[
    ("projection_threads", "thread_id"),
    ("projection_threads", "project_id"),
    ("projection_threads", "title"),
    ("projection_threads", "deleted_at"),
];
const REQUIRED_V2: &[(&str, &str)] = &[
    ("orchestration_v2_projection_threads", "thread_id"),
    ("orchestration_v2_projection_threads", "project_id"),
    ("orchestration_v2_projection_threads", "title"),
    ("orchestration_v2_projection_threads", "deleted_at"),
    ("orchestration_v2_projection_threads", "payload_json"),
];
const REQUIRED_BOTH: &[(&str, &str)] = &[("projection_projects", "project_id"), ("projection_projects", "workspace_root")];

fn mutation() -> impl Strategy<Value = Mutation> {
    prop_oneof![
        proptest::sample::select(vec![
            "projection_threads", "projection_projects", "orchestration_v2_projection_threads", "effect_sql_migrations",
            "provider_session_runtime", "orchestration_v2_projection_provider_threads", "orchestration_v2_legacy_imports",
            "orchestration_v2_projection_messages", "projection_thread_messages",
        ])
        .prop_map(Mutation::DropTable),
        proptest::sample::select(vec![
            ("projection_threads", "title"), ("projection_threads", "deleted_at"), ("projection_threads", "worktree_path"),
            ("projection_threads", "archived_at"), ("projection_projects", "workspace_root"), ("projection_projects", "scripts_json"),
            ("orchestration_v2_projection_threads", "payload_json"), ("orchestration_v2_projection_threads", "title"),
            ("orchestration_v2_projection_threads", "active_provider_thread_id"), ("provider_session_runtime", "resume_cursor_json"),
            ("orchestration_v2_projection_provider_threads", "payload_json"),
        ])
        .prop_map(|(table, column)| Mutation::RenameColumn(table, column)),
        proptest::sample::select(vec!["projection_threads", "orchestration_v2_projection_threads", "projection_projects"]).prop_map(Mutation::AddColumn),
        prop_oneof![Just(i64::MIN), Just(-1i64), Just(0i64), 1i64..200, Just(i64::MAX)].prop_map(Mutation::Version),
        Just(Mutation::ClearMigrations),
        Just(Mutation::WrongTypes),
    ]
}

/// Apply a mutation; `true` when it took effect.
fn mutate(conn: &Connection, mutation: &Mutation) -> bool {
    let sql = match mutation {
        // Triggers and indexes on a table go with it.
        Mutation::DropTable(table) => format!("DROP TABLE {table};"),
        Mutation::RenameColumn(table, column) => format!("ALTER TABLE {table} RENAME COLUMN \"{column}\" TO \"{column}_renamed\";"),
        Mutation::AddColumn(table) => format!("ALTER TABLE {table} ADD COLUMN surprise BLOB;"),
        Mutation::Version(version) => format!("INSERT OR IGNORE INTO effect_sql_migrations(migration_id,name) VALUES({version},'x');"),
        Mutation::ClearMigrations => "DELETE FROM effect_sql_migrations;".into(),
        Mutation::WrongTypes => "UPDATE projection_projects SET workspace_root=7, scripts_json=x'00ff';".into(),
    };
    conn.execute_batch(&sql).is_ok()
}

/// An independent look at whether the reader must read: a known version and
/// every column it needs for that version's era.
fn readable(conn: &Connection) -> bool {
    let version: Option<i64> = conn
        .query_row("SELECT max(migration_id) FROM effect_sql_migrations", [], |row| row.get(0))
        .ok()
        .flatten();
    let Some(version) = version.filter(|version| (OLDEST_MIGRATION..=NEWEST_MIGRATION).contains(version)) else { return false };
    let has = |(table, column): &(&str, &str)| -> bool {
        conn.query_row(&format!("SELECT count(*) FROM pragma_table_info('{table}') WHERE name=?1"), [column], |row| row.get::<_, i64>(0)).unwrap_or(0) == 1
    };
    let era = if version >= 55 { REQUIRED_V2 } else { REQUIRED_V1 };
    era.iter().chain(REQUIRED_BOTH).all(has)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 400, ..ProptestConfig::default() })]

    #[test]
    fn a_mutated_schema_fails_closed_or_reads_correctly(
        version in proptest::sample::select(VERSIONS.to_vec()),
        mutations in proptest::collection::vec(mutation(), 1..4),
        specs in proptest::collection::vec(spec(), 0..6),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let fixture = Fixture::in_base(dir.path(), version);
        fill(&fixture, &specs);
        let applied: Vec<&Mutation> = mutations.iter().filter(|mutation| mutate(&fixture.conn, mutation)).collect();

        let read = store::read(&fixture.conn);
        let expect_read = readable(&fixture.conn);
        prop_assert_eq!(read.is_ok(), expect_read, "{:?} gave {:?}", applied, read);
        match &read {
            Ok(store) => {
                prop_assert!((OLDEST_MIGRATION..=NEWEST_MIGRATION).contains(&store.version));
                for thread in &store.threads {
                    // A table the messages live in may be gone: an error, never a panic.
                    store::messages(&fixture.conn, store.era, thread).unwrap_or_default();
                }
            }
            Err(reason) => prop_assert!(reason.contains("T3 Code"), "{}", reason),
        }
        let shown = format!("{read:?}");
        prop_assert!(!shown.contains(CANARY));
    }
}

/// A store built in memory, for the plan.
fn store_of(threads: Vec<Thread>) -> Store {
    let projects = (0..3)
        .map(|index| {
            let id = format!("p{index}");
            (id.clone(), Project { id, title: None, root: PathBuf::from(format!("/code/repo{index}")), deleted: index == 2, archived: false, scripts: None })
        })
        .collect();
    Store { version: 102, era: Era::V2, projects, threads }
}

fn thread_strategy() -> impl Strategy<Value = Vec<Thread>> {
    proptest::collection::vec(
        (
            0usize..4,
            proptest::option::of((proptest::sample::select(vec!["claude", "codex", "opencode"]), 0u8..6)),
            proptest::option::of(proptest::sample::select(vec!["/w/a", "/w/b", "/w/c"])),
            (any::<bool>(), any::<bool>()),
            proptest::option::of(0usize..10),
        ),
        0..10,
    )
    .prop_map(|rows| {
        let count = rows.len();
        rows.into_iter()
            .enumerate()
            .map(|(index, (project, native, worktree, (archived, deleted), parent))| Thread {
                id: format!("t{index}"),
                project_id: format!("p{project}"),
                title: Some(format!("Thread {index}")),
                worktree: worktree.map(PathBuf::from),
                archived,
                deleted,
                native: native.map(|(harness, id)| NativeSession { harness, id: format!("n{id}") }),
                parent: parent.filter(|parent| *parent < count && *parent != index).map(|parent| format!("t{parent}")),
                updated_at: Some(format!("2026-11-0{}", index % 9 + 1)),
                ..Thread::default()
            })
            .collect()
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2000, ..ProptestConfig::default() })]

    #[test]
    fn every_live_thread_comes_over_exactly_one_way(
        threads in thread_strategy(),
        found in proptest::collection::hash_set((proptest::sample::select(vec!["claude", "codex", "opencode"]), 0u8..6), 0..8),
    ) {
        let store = store_of(threads);
        let found: HashSet<NativeSession> = found.into_iter().map(|(harness, id)| NativeSession { harness, id: format!("n{id}") }).collect();
        let plan = plan_chats(&store, &found);
        let by_id: HashMap<&str, &Thread> = store.threads.iter().map(|thread| (thread.id.as_str(), thread)).collect();

        // Copies are unique and live.
        let copies: HashSet<&str> = plan.copies.iter().map(String::as_str).collect();
        prop_assert_eq!(copies.len(), plan.copies.len(), "a thread copied twice");
        for thread in &store.threads {
            let live = is_live(&store, thread);
            let native = thread.native.as_ref().filter(|native| found.contains(*native));
            let copied = copies.contains(thread.id.as_str());
            if !live {
                prop_assert!(!copied, "{} is deleted", thread.id);
                continue;
            }
            match native {
                Some(native) => {
                    prop_assert!(plan.native.contains(native));
                    prop_assert!(!copied, "{} came over twice", thread.id);
                }
                None => {
                    let with_claude_parent = thread
                        .parent
                        .as_deref()
                        .and_then(|parent| by_id.get(parent))
                        .and_then(|parent| parent.native.as_ref())
                        .is_some_and(|parent| parent.harness == "claude" && found.contains(parent));
                    prop_assert_eq!(copied, !with_claude_parent, "{}", thread.id);
                }
            }
        }
        // Only sessions found are read natively, and only for live threads.
        for native in &plan.native {
            prop_assert!(found.contains(native));
            prop_assert!(store.threads.iter().any(|thread| is_live(&store, thread) && thread.native.as_ref() == Some(native)));
        }

        // Worktrees: once each, from live top-level threads, archived only when all are.
        let candidates = worktree_candidates(&store);
        let paths: HashSet<&Path> = candidates.iter().map(|candidate| candidate.path.as_path()).collect();
        prop_assert_eq!(paths.len(), candidates.len());
        for candidate in &candidates {
            let users: Vec<&Thread> = chats(&store).filter(|thread| thread.worktree.as_deref() == Some(candidate.path.as_path())).collect();
            prop_assert!(!users.is_empty());
            prop_assert_eq!(candidate.archived, users.iter().all(|thread| is_archived(&store, thread)));
        }
        let expected: HashSet<&Path> = chats(&store).filter_map(|thread| thread.worktree.as_deref()).collect();
        prop_assert_eq!(paths, expected);
        prop_assert_eq!(counts(&store).chats, chats(&store).count() as u64);
    }
}

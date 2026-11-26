//! Fuzzing Superset's import (property-based, with `proptest`).
//!
//! - **Store**: random workspaces, chats, terminal sessions, configurations
//!   and journal events, with garbage of every type, on the rebuilt schema
//!   heads. It reads, never panics, queries only allowed tables, never
//!   yields a secret, and reads back what was written.
//! - **Schema mutation**: the newest schemas with columns renamed, tables
//!   dropped and journals shifted. It fails closed or reads correctly
//!   against an independent oracle; it never panics.
//! - **Plan**: random chats and terminal sessions and random sessions found.
//!   Every live native session comes over exactly one way, the rest never,
//!   and each worktree is a candidate once.
//! - **Parsers**: journal items and project configs of any shape never panic.

use super::fixture::{agent, user, ChatRow, Fixture, WorkspaceRow, CANARY, CHAT_HEADS, HOST_HEADS};
use super::store::{self, Body, Kind, NativeSession};
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
        Just(Sql::Text("{\"type\":\"item\",\"item\":{\"id\":\"i\",\"kind\":\"agent_message\",\"text\":\"hi\"}}".into())),
        Just(Sql::Text("{\"ANTHROPIC_API_KEY\":\"CANARY-superset-secret\"}".into())),
        Just(Sql::Text("not json".into())),
        proptest::collection::vec(any::<u8>(), 0..8).prop_map(Sql::Blob),
    ]
}

/// A random JSON value, nested a little.
fn any_json() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|number| json!(number)),
        "[ -~]{0,12}".prop_map(Value::String),
    ];
    leaf.prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
            proptest::collection::btree_map(
                proptest::sample::select(vec!["id", "kind", "text", "content", "type", "hidden", "parentItemId", "discarded", "toolName", "title", "status", "rawInput", "rawOutput", "output", "summary", "startedAtMs"]),
                inner,
                0..6
            )
            .prop_map(|map| Value::Object(map.into_iter().map(|(key, value)| (key.to_owned(), value)).collect())),
        ]
    })
}

const TREES: &[&str] = &["/h/.paradev/worktrees/shop/a", "/h/.paradev/worktrees/shop/b", "/code/shop", "/h/.paradev/sessions/s"];
const KINDS: &[&str] = &["worktree", "worktree", "local", "session"];

#[derive(Debug, Clone)]
struct Spec {
    id: String,
    harness: &'static str,
    workspace: usize,
    native: Option<String>,
    texts: Vec<(bool, String)>,
}

fn spec() -> impl Strategy<Value = Spec> {
    (
        "[a-z0-9-]{1,10}",
        proptest::sample::select(vec!["opencode-acp", "claude-acp", "codex", "pi-acp", "gemini-acp"]),
        0usize..4,
        proptest::option::of("[A-Za-z0-9_-]{1,20}"),
        proptest::collection::vec((any::<bool>(), "[ -~]{0,14}"), 0..4),
    )
        .prop_map(|(id, harness, workspace, native, texts)| Spec { id, harness, workspace, native, texts })
}

fn unique(specs: Vec<Spec>) -> Vec<Spec> {
    let mut seen = HashSet::new();
    specs.into_iter().filter(|spec| seen.insert(spec.id.clone())).collect()
}

fn fill(fixture: &Fixture, specs: &[Spec], archived: bool, host_head: i64) {
    fixture.project("p1", Path::new("/code/shop"), "shop");
    for (index, path) in TREES.iter().enumerate() {
        let id = format!("w{index}");
        // Project-less workspaces arrived with 0020.
        let project = if KINDS[index] == "session" && host_head >= 20 { None } else { Some("p1") };
        fixture.workspace(&WorkspaceRow { id: &id, project, path: Path::new(path), branch: "b", name: "n", kind: KINDS[index], archived: archived && index == 1 });
    }
    for spec in specs {
        let workspace = format!("w{}", spec.workspace);
        fixture.chat(&ChatRow { id: &spec.id, workspace: &workspace, harness: spec.harness, native: spec.native.as_deref(), ..ChatRow::default() });
        let events: Vec<Value> = spec
            .texts
            .iter()
            .enumerate()
            .map(|(index, (mine, text))| if *mine { user(&format!("{}-{index}", spec.id), text) } else { agent(&format!("{}-{index}", spec.id), text) })
            .collect();
        fixture.events(&spec.id, "e1", &events);
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 200, ..ProptestConfig::default() })]

    /// Random chats on the rebuilt heads read back as written; garbage in
    /// any column never panics, never leaks, and only allowed tables are read.
    #[test]
    fn random_rows_on_the_rebuilt_heads_read_safely(
        host_head in proptest::sample::select(HOST_HEADS.to_vec()),
        chat_head in proptest::sample::select(CHAT_HEADS.to_vec()),
        specs in proptest::collection::vec(spec(), 0..6).prop_map(unique),
        archived in any::<bool>(),
        garbage in proptest::collection::vec((0usize..5, any_value()), 0..6),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let fixture = Fixture::at(dir.path(), host_head, Some(chat_head));
        fill(&fixture, &specs, archived, host_head);
        STATEMENTS.with(|statements| statements.borrow_mut().clear());
        let store = read(&Paths::at(dir.path())).unwrap();
        // The oracle: what was written, read independently of the reader.
        let org = &store.orgs[0];
        let expected: BTreeSet<&str> = specs.iter().map(|spec| spec.id.as_str()).collect();
        let got: BTreeSet<&str> = org.chats.iter().map(|chat| chat.id.as_str()).collect();
        prop_assert_eq!(&got, &expected);
        for spec in &specs {
            let chat = org.chats.iter().find(|chat| chat.id == spec.id).unwrap();
            let want = store::harness_of_chat(spec.harness).zip(spec.native.clone()).map(|(harness, id)| NativeSession { harness, id });
            prop_assert_eq!(&chat.native, &want);
            prop_assert_eq!(is_live(org, chat), store::harness_of_chat(spec.harness).is_some());
            let items = store::items(fixture.chat_conn(), &chat.id, "e1").unwrap();
            prop_assert_eq!(items.len(), spec.texts.iter().filter(|(_, text)| !text.trim().is_empty()).count());
            prop_assert_eq!(is_archived(org, &chat.workspace), archived && spec.workspace == 1 && host_head >= 20);
        }
        let candidates = worktree_candidates(&store);
        // Before workspaces had a type (0017) the session folder read as a worktree too; the project's checkout never does.
        prop_assert_eq!(candidates.len(), if host_head >= 17 { 2 } else { 3 }, "{:?}", candidates);

        for (target, value) in &garbage {
            let (conn, sql) = match target {
                0 => (&fixture.host, "UPDATE workspaces SET worktree_path = ?1"),
                1 => (fixture.chat_conn(), "UPDATE chat_journal SET event_json = ?1"),
                2 => (fixture.chat_conn(), "UPDATE chat_sessions_local SET harness_session_id = ?1"),
                3 => (&fixture.host, "UPDATE host_agent_configs SET env_json = ?1"),
                _ => (&fixture.host, "UPDATE projects SET repo_path = ?1"),
            };
            // NOT NULL columns refuse a null, and old heads lack some tables; that is fine.
            conn.execute(sql, params![value]).unwrap_or_default();
        }
        if let Ok(store) = read(&Paths::at(dir.path())) {
            for (org, chat) in chats(&store) {
                let items = store::items(fixture.chat_conn(), &chat.id, "e1").unwrap_or_default();
                let shown = format!("{:?}", copy_chat(org, chat, &items, "ref"));
                prop_assert!(!shown.contains(CANARY));
            }
            let shown = format!("{:?}{:?}", store, own_configs(&store).map(|(_, config)| config_setup(config, "ref")).collect::<Vec<_>>());
            prop_assert!(!shown.contains(CANARY));
        }
        let allowed: Vec<&str> = store::HOST_TABLES.iter().chain(store::CHAT_TABLES).copied().collect();
        for statement in STATEMENTS.with(|statements| statements.borrow().clone()) {
            prop_assert!(allowed.iter().any(|table| statement.contains(&format!("FROM {table}")) || statement.contains(&format!("('{table}')"))), "{}", statement);
            prop_assert!(!statement.contains("SELECT \"env_json\""), "{}", statement);
        }
    }
}

#[derive(Debug, Clone)]
enum Mutation {
    RenameHost(&'static str, &'static str),
    RenameChat(&'static str, &'static str),
    DropHost(&'static str),
    DropChat(&'static str),
    HostJournal(i64),
    ChatJournal(i64),
}

fn mutation() -> impl Strategy<Value = Mutation> {
    prop_oneof![
        proptest::sample::select(vec![
            ("projects", "repo_path"),
            ("projects", "deleted_at"),
            ("workspaces", "worktree_path"),
            ("workspaces", "type"),
            ("workspaces", "archived_at"),
            ("terminal_agent_bindings", "agent_session_id"),
            ("host_agent_configs", "env_json"),
            ("host_settings", "default_codex_home"),
            ("workspace_pull_requests", "workspace_id"),
        ])
        .prop_map(|(table, column)| Mutation::RenameHost(table, column)),
        proptest::sample::select(vec![
            ("chat_sessions_local", "scope_id"), ("chat_sessions_local", "harness_session_id"), ("chat_sessions_local", "title"),
            ("chat_journal", "event_json"), ("chat_journal", "epoch"),
        ])
            .prop_map(|(table, column)| Mutation::RenameChat(table, column)),
        proptest::sample::select(vec!["terminal_agent_bindings", "host_agent_configs", "workspace_tags", "host_settings", "workspaces"]).prop_map(Mutation::DropHost),
        proptest::sample::select(vec!["chat_journal", "chat_sessions_local"]).prop_map(Mutation::DropChat),
        prop_oneof![
            Just(store::HOST_MIGRATIONS.newest_checked + 1), Just(store::HOST_MIGRATIONS.oldest - 1), Just(store::HOST_MIGRATIONS.newest_checked),
            Just(store::HOST_MIGRATIONS.oldest), Just(i64::MAX),
        ]
        .prop_map(Mutation::HostJournal),
        prop_oneof![
            Just(store::CHAT_MIGRATIONS.newest_checked + 1), Just(store::CHAT_MIGRATIONS.oldest - 1), Just(store::CHAT_MIGRATIONS.newest_checked),
            Just(store::CHAT_MIGRATIONS.oldest), Just(i64::MAX),
        ]
        .prop_map(Mutation::ChatJournal),
    ]
}

/// Apply a mutation; a second rename or drop of the same thing does nothing.
fn apply(fixture: &Fixture, mutation: &Mutation) {
    let rename = |conn: &Connection, table: &str, column: &str| {
        let _ = conn.execute_batch(&format!("ALTER TABLE {table} RENAME COLUMN {column} TO {column}_gone"));
    };
    let journal = |conn: &Connection, when: i64| {
        conn.execute("DELETE FROM __drizzle_migrations", []).unwrap();
        conn.execute("INSERT INTO __drizzle_migrations(hash, created_at) VALUES('h', ?1)", [when]).unwrap();
    };
    match mutation {
        Mutation::RenameHost(table, column) => rename(&fixture.host, table, column),
        Mutation::RenameChat(table, column) => rename(fixture.chat_conn(), table, column),
        Mutation::DropHost(table) => fixture.host.execute_batch(&format!("DROP TABLE IF EXISTS {table}")).unwrap(),
        Mutation::DropChat(table) => fixture.chat_conn().execute_batch(&format!("DROP TABLE IF EXISTS {table}")).unwrap(),
        Mutation::HostJournal(when) => journal(&fixture.host, *when),
        Mutation::ChatJournal(when) => journal(fixture.chat_conn(), *when),
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 300, ..ProptestConfig::default() })]

    /// A changed schema fails closed or reads correctly; it never panics.
    #[test]
    fn a_mutated_schema_fails_closed_or_reads_correctly(
        specs in proptest::collection::vec(spec(), 1..5).prop_map(unique),
        mutations in proptest::collection::vec(mutation(), 1..4),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let fixture = Fixture::newest(dir.path());
        fill(&fixture, &specs, true, 38);
        fixture.binding("t1", "w0", "claude", Some("11111111-1111-4111-8111-111111111111"), None);
        for mutation in &mutations {
            apply(&fixture, mutation);
        }
        let result = read(&Paths::at(dir.path()));
        // The independent oracle, asked of SQLite: what a correct reader must do.
        let fails = !readable(&fixture.host, store::HOST_MIGRATIONS, &[("projects", "id"), ("projects", "repo_path"), ("workspaces", "id"), ("workspaces", "worktree_path")], store::HOST_NEWEST_READS)
            || !readable(fixture.chat_conn(), store::CHAT_MIGRATIONS, &[("chat_sessions_local", "session_id"), ("chat_sessions_local", "scope_id"), ("chat_sessions_local", "harness")], store::CHAT_NEWEST_READS);
        prop_assert_eq!(result.is_err(), fails, "{:?} gave {:?}", mutations, result.as_ref().err());
        if let Err(reason) = &result {
            prop_assert!(reason.contains("Superset"), "{}", reason);
        }
        let mutation = |wanted: &Mutation| mutations.iter().any(|mutation| format!("{mutation:?}") == format!("{wanted:?}"));
        if let Ok(store) = result {
            let org = &store.orgs[0];
            let got: BTreeSet<&str> = org.chats.iter().map(|chat| chat.id.as_str()).collect();
            let expected: BTreeSet<&str> = specs.iter().map(|spec| spec.id.as_str()).collect();
            prop_assert_eq!(got, expected);
            let bindings_read = !mutation(&Mutation::DropHost("terminal_agent_bindings"));
            prop_assert_eq!(org.host.bindings.len(), usize::from(bindings_read));
            if mutation(&Mutation::RenameHost("host_agent_configs", "env_json")) {
                prop_assert!(org.host.configs.iter().all(|config| config.env_names.is_empty()));
            }
            if mutation(&Mutation::RenameHost("workspaces", "archived_at")) {
                prop_assert!(org.host.workspaces.values().all(|workspace| !workspace.archived));
            }
            if mutation(&Mutation::RenameHost("workspaces", "type")) {
                prop_assert!(org.host.workspaces.values().all(|workspace| workspace.kind == Kind::Worktree), "an old head without types: all worktrees");
            }
            for chat in &org.chats {
                store::items(fixture.chat_conn(), &chat.id, "e1").unwrap_or_default();
            }
        }
    }
}

/// Whether a drizzle database must be read: a version no older than the
/// oldest, the columns always needed, and for a version newer than the
/// newest checked every column of `newest_reads`.
fn readable(conn: &Connection, versions: crate::import_sqlite::Versions, needed: &[(&str, &str)], newest_reads: crate::import_sqlite::Reads) -> bool {
    let has = |table: &str, column: &str| -> bool {
        conn.query_row(&format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name=?1"), [column], |row| row.get::<_, i64>(0)).unwrap_or(0) > 0
    };
    let version: Option<i64> = conn.query_row("SELECT MAX(created_at) FROM __drizzle_migrations", [], |row| row.get(0)).unwrap_or(None);
    let Some(version) = version.filter(|version| *version >= versions.oldest) else { return false };
    needed.iter().all(|(table, column)| has(table, column))
        && (version <= versions.newest_checked || newest_reads.iter().all(|(table, columns)| columns.iter().all(|column| has(table, column))))
}

fn store_strategy() -> impl Strategy<Value = Store> {
    let chats = proptest::collection::vec(
        ("[a-z0-9]{1,6}", proptest::sample::select(vec!["opencode-acp", "claude-code", "pi-acp"]), proptest::option::of("[a-z0-9]{1,3}"), 0usize..4),
        0..10,
    );
    let bindings = proptest::collection::vec(
        ("[a-z0-9]{1,6}", proptest::sample::select(vec!["opencode", "claude", "amp"]), proptest::option::of("[a-z0-9]{1,3}"), 0usize..4),
        0..8,
    );
    (chats, bindings, any::<bool>()).prop_map(|(chats, bindings, deleted)| {
        let mut host = Host { version: store::HOST_MIGRATIONS.newest_checked, ..Host::default() };
        host.projects.insert("p1".into(), store::Project { id: "p1".into(), root: PathBuf::from("/code/shop"), name: None, deleted });
        for (index, path) in TREES.iter().enumerate() {
            let kind = match KINDS[index] {
                "worktree" => Kind::Worktree,
                "local" => Kind::Local,
                _ => Kind::Session,
            };
            let project = (kind != Kind::Session).then(|| "p1".to_owned());
            host.workspaces.insert(format!("w{index}"), store::Workspace { id: format!("w{index}"), project, path: PathBuf::from(path), branch: None, name: None, kind, archived: index == 1 });
        }
        let mut seen = HashSet::new();
        host.bindings = bindings
            .into_iter()
            .filter(|(terminal, ..)| seen.insert(terminal.clone()))
            .map(|(terminal, agent, session, workspace)| store::Binding {
                native: store::harness_of_agent(agent).zip(session).map(|(harness, id)| NativeSession { harness, id }),
                terminal,
                workspace: format!("w{workspace}"),
                agent: agent.into(),
                transcript: None,
            })
            .collect();
        let mut seen = HashSet::new();
        let chats = chats
            .into_iter()
            .filter(|(id, ..)| seen.insert(id.clone()))
            .map(|(id, harness, session, workspace)| store::Chat {
                native: store::harness_of_chat(harness).zip(session).map(|(harness, id)| NativeSession { harness, id }),
                workspace: format!("w{workspace}"),
                harness: harness.into(),
                epoch: None,
                title: None,
                updated_at: None,
                id,
            })
            .collect();
        Store { orgs: vec![Organization { id: "o".into(), host_path: PathBuf::from("/h/host/o/host.db"), chat_path: Some(PathBuf::from("/h/host/o/chat.db")), host, chat_version: Some(store::CHAT_MIGRATIONS.newest_checked), chats }] }
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2000, ..ProptestConfig::default() })]

    /// Every live native session comes over exactly one way: natively when
    /// found, else (for a chat) as Superset's copy, else (for a terminal)
    /// counted as gone. Nothing else ever comes over.
    #[test]
    fn every_live_session_comes_over_exactly_one_way(
        store in store_strategy(),
        found_ids in proptest::collection::hash_set("[a-z0-9]{1,3}", 0..6),
    ) {
        let found: HashSet<NativeSession> = wanted(&store).into_iter().filter(|native| found_ids.contains(&native.id)).collect();
        let plan = plan_chats(&store, &found);
        let org = &store.orgs[0];
        let deleted = org.host.projects["p1"].deleted;
        for chat in &org.chats {
            let live = store::harness_of_chat(&chat.harness).is_some() && (!deleted || chat.workspace == "w3");
            let copied = plan.copies.contains(&(org.id.clone(), chat.id.clone()));
            let native = chat.native.as_ref().is_some_and(|native| found.contains(native) && plan.native.contains(native));
            if live {
                prop_assert!(copied != native, "{:?} comes over exactly one way", chat);
            } else {
                prop_assert!(!copied, "{:?} never comes over", chat);
            }
        }
        let terminals: Vec<&store::Binding> = bindings(&store).map(|(_, binding)| binding).collect();
        let gone = terminals.iter().filter(|binding| !binding.native.as_ref().is_some_and(|native| found.contains(native))).count();
        prop_assert_eq!(plan.gone, gone);
        let chat_natives: HashSet<&NativeSession> = chats(&store).filter_map(|(_, chat)| chat.native.as_ref()).collect();
        prop_assert!(terminals.iter().all(|binding| binding.native.as_ref().is_some_and(|native| !chat_natives.contains(native))), "a terminal a chat names is that chat");
        prop_assert!(plan.native.iter().all(|native| found.contains(native)));
        let candidates = worktree_candidates(&store);
        let paths: HashSet<&PathBuf> = candidates.iter().map(|candidate| &candidate.path).collect();
        prop_assert_eq!(paths.len(), candidates.len());
        prop_assert_eq!(candidates.len(), if deleted { 0 } else { 2 });
    }

    /// A journal item of any shape never panics, and only conversation of
    /// this chat's own becomes an entry.
    #[test]
    fn any_journal_item_reads_safely(item in any_json()) {
        if let Some(read) = store::item_of(&item) {
            prop_assert!(matches!(item.get("kind").and_then(Value::as_str), Some("user_message" | "agent_message" | "reasoning" | "tool_call")));
            prop_assert!(item.get("parentItemId").is_none_or(Value::is_null));
            if let Body::User(text) | Body::Assistant(text) | Body::Reasoning(text) = &read.body {
                prop_assert!(!text.trim().is_empty());
                prop_assert!(!text.contains("<superset-context>"));
            }
        }
    }

    /// Project configs of any shape never panic.
    #[test]
    fn project_configs_never_panic(raw in "[ -~\n]{0,200}", value in any_json()) {
        let _ = store::json_scripts(&raw);
        let _ = store::toml_scripts(&raw);
        let _ = store::json_scripts(&value.to_string());
    }
}

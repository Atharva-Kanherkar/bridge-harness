//! Fuzzing Orca's import (property-based, with `proptest`).
//!
//! - **Store**: random records, documents, journal rows and runs, with
//!   random values and types, on the real schemas. It reads, never panics,
//!   queries only allowed tables, never yields a secret, and reads back what
//!   was written.
//! - **Schema mutation**: the real schemas with columns renamed, tables
//!   dropped and versions shifted. It fails closed or reads correctly
//!   against an independent oracle; it never panics.
//! - **Plan**: random chat sets and random sets of sessions found. Every live
//!   chat comes over exactly one way, the rest never, and each worktree is a
//!   candidate once.
//! - **Parsers**: worktree ids, `orca.yaml` and journal payloads never panic,
//!   and known forms round-trip.

use super::fixture::{message, worktree_id, ChatRow, Fixture, Handle, CANARY};
use super::store::{self, Body, NativeSession, Place, Worker};
use super::*;
use crate::import_sqlite::STATEMENTS;
use proptest::prelude::*;
use rusqlite::types::Value as Sql;
use serde_json::json;

/// A random value of any SQLite type, garbage included.
fn any_value() -> impl Strategy<Value = Sql> {
    prop_oneof![
        Just(Sql::Null),
        any::<i64>().prop_map(Sql::Integer),
        any::<f64>().prop_map(Sql::Real),
        "[ -~]{0,24}".prop_map(Sql::Text),
        Just(Sql::Text("{\"sessionId\":\"x\",\"providerHandleChain\":[{\"handle\":{\"provider\":\"claude\",\"sessionId\":\"s\"}}]}".into())),
        Just(Sql::Text("{\"body\":{\"kind\":\"message\",\"role\":\"user\",\"blocks\":[{\"type\":\"text\",\"text\":\"hi\"}]},\"itemId\":\"i\"}".into())),
        Just(Sql::Text("not json".into())),
        Just(Sql::Text("[1,2,3]".into())),
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
                proptest::sample::select(vec!["kind", "role", "blocks", "type", "text", "itemId", "agentId", "body", "name", "input", "output", "head", "state", "callId", "observedAt"]),
                inner,
                0..5
            )
            .prop_map(|map| Value::Object(map.into_iter().map(|(key, value)| (key.to_owned(), value)).collect())),
        ]
    })
}

#[derive(Debug, Clone)]
struct Spec {
    id: String,
    agent: &'static str,
    worktree: Option<usize>,
    handle: Option<(u8, String)>,
    host: &'static str,
    title: Option<String>,
    deleted: bool,
    items: Vec<(&'static str, String)>,
}

const TREES: &[&str] = &["/w/orca/workspaces/shop/a", "/w/orca/workspaces/shop/b", "/w/code/shop"];

fn spec() -> impl Strategy<Value = Spec> {
    (
        "[a-z0-9-]{1,12}",
        proptest::sample::select(vec!["claude", "codex", "opencode", "pi", "gemini", ""]),
        proptest::option::of(0usize..3),
        proptest::option::of((0u8..4, "[A-Za-z0-9_-]{1,24}")),
        proptest::sample::select(vec!["local", "local", "ssh:box"]),
        proptest::option::of("[ -~]{0,20}"),
        any::<bool>(),
        proptest::collection::vec((proptest::sample::select(vec!["user", "assistant", "reasoning", "system"]), "[ -~]{0,16}"), 0..4),
    )
        .prop_map(|(id, agent, worktree, handle, host, title, deleted, items)| Spec { id, agent, worktree, handle, host, title, deleted, items })
}

fn fill(fixture: &Fixture, specs: &[Spec]) {
    fixture.repos("local-default", &[("r1", Path::new(TREES[2]))]);
    let meta: Vec<(String, &str, bool)> = TREES.iter().enumerate().map(|(index, tree)| (worktree_id("r1", Path::new(tree)), "", index == 1)).collect();
    fixture.worktree_meta("local-default", &meta);
    for spec in specs {
        let handle = spec.handle.as_ref().map(|(form, id)| match form {
            0 => Handle::Claude(id),
            1 => Handle::Codex(id),
            2 => Handle::Acp("opencode", id),
            _ => Handle::Acp("pi", id),
        });
        fixture.chat(&ChatRow {
            id: &spec.id,
            agent: spec.agent,
            workspace: spec.worktree.map_or_else(|| "global-floating-terminal".to_owned(), |index| worktree_id("r1", Path::new(TREES[index]))),
            host: spec.host,
            title: spec.title.as_deref(),
            handles: handle.into_iter().collect(),
            ..ChatRow::default()
        });
        if spec.deleted {
            fixture.delete_chat(&spec.id);
        }
        let payloads: Vec<Value> =
            spec.items.iter().enumerate().map(|(index, (role, text))| message(&format!("{}-{index}", spec.id), role, text, 1_795_419_160_000)).collect();
        fixture.chats.execute("DELETE FROM journal_items WHERE session_id=?1", params![spec.id]).unwrap();
        fixture.items(&spec.id, "e1", &payloads);
    }
}

fn unique(specs: Vec<Spec>) -> Vec<Spec> {
    let mut seen = HashSet::new();
    specs.into_iter().filter(|spec| seen.insert(spec.id.clone())).collect()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 300, ..ProptestConfig::default() })]

    /// Random chats on the real schemas read back as written; garbage in
    /// any column never panics, never leaks, and only allowed tables are read.
    #[test]
    fn random_rows_on_the_real_schemas_read_safely(
        specs in proptest::collection::vec(spec(), 0..8).prop_map(unique),
        garbage in proptest::collection::vec((0usize..4, any_value()), 0..6),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let fixture = Fixture::at(dir.path());
        fill(&fixture, &specs);
        STATEMENTS.with(|statements| statements.borrow_mut().clear());
        let (store, _) = read(&Paths::at(dir.path())).unwrap();
        // The oracle: what was written, read independently of the reader.
        let expected: BTreeSet<&str> = specs.iter().filter(|spec| !spec.deleted).map(|spec| spec.id.as_str()).collect();
        let got: BTreeSet<&str> = store.chats.iter().map(|chat| chat.id.as_str()).collect();
        prop_assert_eq!(&got, &expected);
        for spec in specs.iter().filter(|spec| !spec.deleted) {
            let chat = store.chats.iter().find(|chat| chat.id == spec.id).unwrap();
            prop_assert_eq!(chat.local, spec.host == "local");
            prop_assert_eq!(chat.title.as_deref(), spec.title.as_deref().map(str::trim).filter(|title| !title.is_empty()));
            let expected_native = spec.handle.as_ref().and_then(|(form, id)| match form {
                0 => Some(NativeSession { harness: "claude", id: id.clone() }),
                1 => Some(NativeSession { harness: "codex", id: id.clone() }),
                2 => Some(NativeSession { harness: "opencode", id: id.clone() }),
                _ => None,
            });
            prop_assert_eq!(&chat.native, &expected_native);
            let items = store::items(&fixture.chats, &chat.id).unwrap();
            let expected_items = spec.items.iter().filter(|(role, text)| *role != "system" && !text.trim().is_empty()).count();
            prop_assert_eq!(items.len(), expected_items);
        }
        let candidates = worktree_candidates(&store);
        let paths: HashSet<&PathBuf> = candidates.iter().map(|candidate| &candidate.path).collect();
        prop_assert_eq!(paths.len(), candidates.len(), "each worktree once");
        prop_assert!(candidates.iter().all(|candidate| candidate.path != Path::new(TREES[2])), "never the main checkout");

        // Garbage written over the real rows.
        for (target, value) in &garbage {
            let sql = match target {
                0 => "UPDATE agent_session_records SET record_json = ?1",
                1 => "UPDATE journal_items SET payload_json = ?1",
                2 => "UPDATE journal_chat_state SET epoch = ?1",
                _ => "UPDATE agent_session_store_meta SET value = ?1",
            };
            // NOT NULL columns refuse a null; that is fine.
            fixture.chats.execute(sql, params![value]).unwrap_or_default();
        }
        if let Ok((store, path)) = read(&Paths::at(dir.path())) {
            let links = links(&store);
            for chat in chats(&store) {
                let items = store::items(&fixture.chats, &chat.id).unwrap_or_default();
                let copy = copy_chat(&store, chat, &items, &source_ref(&path, &chat.id), &links);
                let shown = format!("{:?}", copy);
                prop_assert!(!shown.contains(CANARY));
            }
            let shown = format!("{:?}", store);
            prop_assert!(!shown.contains(CANARY));
        }
        let allowed: Vec<&str> = store::CHAT_TABLES.iter().chain(store::PROFILE_TABLES).chain(store::ORCHESTRATION_TABLES).copied().collect();
        for statement in STATEMENTS.with(|statements| statements.borrow().clone()) {
            prop_assert!(statement == "PRAGMA user_version" || allowed.iter().any(|table| statement.contains(&format!("FROM {table}")) || statement.contains(&format!("('{table}')"))), "{}", statement);
        }
    }
}

#[derive(Debug, Clone)]
enum Mutation {
    RenameColumn(&'static str, &'static str),
    DropTable(&'static str),
    ChatVersion(i64),
    ProfileVersion(i64),
    RunColumn(&'static str, &'static str),
}

fn mutation() -> impl Strategy<Value = Mutation> {
    prop_oneof![
        proptest::sample::select(vec![
            ("agent_session_records", "record_json"),
            ("agent_session_records", "session_id"),
            ("journal_items", "payload_json"),
            ("journal_items", "hidden"),
            ("journal_chat_state", "epoch"),
            ("agent_session_store_meta", "value"),
            ("journal_items", "ordinal"),
            ("journal_items", "item_id"),
        ])
        .prop_map(|(table, column)| Mutation::RenameColumn(table, column)),
        proptest::sample::select(vec!["agent_session_records", "journal_items", "journal_chat_state", "agent_session_store_meta"]).prop_map(Mutation::DropTable),
        (-2i64..6).prop_map(Mutation::ChatVersion),
        (-2i64..7).prop_map(Mutation::ProfileVersion),
        proptest::sample::select(vec![
            ("runs", "coordinator_orca_session_id"),
            ("dispatch_contexts", "run_id"),
            ("worker_terminal_resources", "resume_session"),
            ("worker_terminal_resources", "origin_dispatch_id"),
        ])
        .prop_map(|(table, column)| Mutation::RunColumn(table, column)),
    ]
}

fn apply(fixture: &Fixture, mutation: &Mutation) {
    // A second rename or drop of the same thing does nothing.
    let rename = |conn: &Connection, table: &str, column: &str| {
        drop(conn.execute_batch(&format!("ALTER TABLE {table} RENAME COLUMN {column} TO {column}_gone")));
    };
    match mutation {
        Mutation::RenameColumn(table, column) => {
            // Generated columns and triggers name some columns; drop them first.
            fixture.chats.execute_batch("PRAGMA writable_schema=OFF;").unwrap();
            let triggers: Vec<String> = fixture
                .chats
                .prepare("SELECT name FROM sqlite_master WHERE type='trigger'")
                .unwrap()
                .query_map([], |row| row.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            for trigger in triggers {
                fixture.chats.execute_batch(&format!("DROP TRIGGER \"{trigger}\"")).unwrap();
            }
            if *column == "record_json" {
                // `adopted_native_id` is generated from it.
                fixture.chats.execute_batch("DROP INDEX IF EXISTS idx_agent_session_records_adopted_native_id").ok();
                drop(fixture.chats.execute_batch("BEGIN; ALTER TABLE agent_session_records RENAME TO old_records; CREATE TABLE agent_session_records(session_id TEXT PRIMARY KEY, record_json_gone TEXT); INSERT INTO agent_session_records SELECT session_id, record_json FROM old_records; DROP TABLE old_records; COMMIT;"));
                drop(fixture.chats.execute_batch("ROLLBACK"));
            } else {
                rename(&fixture.chats, table, column);
            }
        }
        Mutation::DropTable(table) => fixture.chats.execute_batch(&format!("DROP TABLE IF EXISTS {table}")).unwrap(),
        Mutation::ChatVersion(version) => fixture.chats.execute_batch(&format!("PRAGMA user_version = {version}")).unwrap(),
        Mutation::ProfileVersion(version) => fixture.profile("local-default").execute_batch(&format!("PRAGMA user_version = {version}")).unwrap(),
        Mutation::RunColumn(table, column) => rename(&fixture.runs, table, column),
    }
}

/// Whether a database must be read: a `user_version` no older than the
/// oldest, the columns always needed, and for a version newer than the
/// newest checked every column of `newest_reads`.
fn readable(conn: &Connection, versions: crate::import_sqlite::Versions, needed: &[(&str, &str)], newest_reads: crate::import_sqlite::Reads) -> bool {
    let has = |table: &str, column: &str| -> bool {
        conn.query_row(&format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name=?1"), [column], |row| row.get::<_, i64>(0)).unwrap_or(0) > 0
    };
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0)).unwrap();
    version >= versions.oldest
        && needed.iter().all(|(table, column)| has(table, column))
        && (version <= versions.newest_checked || newest_reads.iter().all(|(table, columns)| columns.iter().all(|column| has(table, column))))
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 300, ..ProptestConfig::default() })]

    /// A changed schema fails closed or reads correctly; never panics, never
    /// returns chats it could not read whole.
    #[test]
    fn a_mutated_schema_fails_closed_or_reads_correctly(
        specs in proptest::collection::vec(spec(), 1..5).prop_map(unique),
        mutations in proptest::collection::vec(mutation(), 1..4),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let fixture = Fixture::at(dir.path());
        fill(&fixture, &specs);
        fixture.run("run_1", Some(&specs[0].id));
        fixture.terminal_worker("run_1", "ctx_1", "claude", Some("s1"));
        for mutation in &mutations {
            apply(&fixture, mutation);
        }
        let result = read(&Paths::at(dir.path()));
        // The independent oracle, asked of SQLite: what a correct reader must do.
        let fails = !readable(&fixture.chats, store::CHAT_VERSIONS, &[("agent_session_records", "session_id"), ("agent_session_records", "record_json")], store::CHAT_NEWEST_READS)
            || !readable(&fixture.profile("local-default"), store::PROFILE_VERSIONS, &[("profile_state_documents", "domain"), ("profile_state_documents", "payload"), ("profile_state_documents", "revision")], store::PROFILE_NEWEST_READS);
        prop_assert_eq!(result.is_err(), fails, "{:?} gave {:?}", mutations, result.as_ref().err());
        let any = |wanted: &dyn Fn(&Mutation) -> bool| mutations.iter().any(wanted);
        if let Ok((store, _)) = result {
            let meta_gone = any(&|mutation| matches!(mutation, Mutation::DropTable("agent_session_store_meta") | Mutation::RenameColumn("agent_session_store_meta", _)));
            let expected: BTreeSet<&str> = specs
                .iter()
                .filter(|spec| !spec.deleted || meta_gone)
                .map(|spec| spec.id.as_str())
                .collect();
            let got: BTreeSet<&str> = store.chats.iter().map(|chat| chat.id.as_str()).collect();
            if !meta_gone {
                prop_assert_eq!(got, expected);
            }
            let run_readable = !any(&|mutation| matches!(mutation, Mutation::RunColumn("runs", _) | Mutation::RunColumn("dispatch_contexts", _)));
            prop_assert_eq!(store.runs.is_some(), run_readable);
            for chat in &store.chats {
                // Missing journal tables or columns read as no copy, or fail with a reason.
                store::items(&fixture.chats, &chat.id).unwrap_or_default();
            }
        }
    }
}

fn chat_strategy() -> impl Strategy<Value = Vec<Chat>> {
    proptest::collection::vec(
        (
            "[a-z0-9]{1,6}",
            proptest::sample::select(vec!["claude", "codex", "opencode", "pi"]),
            proptest::option::of(("[a-z0-9]{1,4}", proptest::sample::select(vec!["claude", "codex", "opencode"]))),
            any::<bool>(),
            proptest::option::of(0usize..3),
        ),
        0..12,
    )
    .prop_map(|rows| {
        let mut seen = HashSet::new();
        rows.into_iter()
            .filter(|(id, ..)| seen.insert(id.clone()))
            .map(|(id, agent, native, local, tree)| Chat {
                place: tree.map_or(Place::None, |index| Place::Worktree { repo: "r1".into(), path: PathBuf::from(TREES[index]) }),
                native: native.map(|(id, harness)| NativeSession { harness, id }),
                agent: agent.into(),
                local,
                launch_directory: None,
                title: None,
                renamed: false,
                created_at: None,
                updated_at: None,
                model: None,
                account_home: None,
                id,
            })
            .collect()
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2000, ..ProptestConfig::default() })]

    /// Every live chat comes over exactly one way: natively when its session
    /// was found, else as Orca's copy. Others never.
    #[test]
    fn every_live_chat_comes_over_exactly_one_way(
        chats_in in chat_strategy(),
        found_ids in proptest::collection::hash_set("[a-z0-9]{1,4}", 0..6),
        runs in proptest::collection::vec((proptest::option::of(0usize..12), proptest::collection::vec(0usize..12, 0..4)), 0..3),
    ) {
        let found: HashSet<NativeSession> = chats_in
            .iter()
            .filter_map(|chat| chat.native.clone())
            .filter(|native| found_ids.contains(&native.id))
            .collect();
        let runs: Vec<store::Run> = runs
            .into_iter()
            .enumerate()
            .map(|(index, (lead, workers))| store::Run {
                id: format!("run_{index}"),
                objective: None,
                coordinator: lead.and_then(|lead| chats_in.get(lead)).map(|chat| chat.id.clone()),
                workers: workers.into_iter().filter_map(|worker| chats_in.get(worker)).map(|chat| Worker::Chat(chat.id.clone())).collect(),
            })
            .collect();
        let store = Store { chat_version: 2, chats: chats_in.clone(), profiles: Vec::new(), runs: Some(runs), notes: Vec::new() };
        let plan = plan_chats(&store, &found);
        for chat in &chats_in {
            let copied = plan.copies.contains(&chat.id);
            let native = chat.native.as_ref().is_some_and(|native| plan.native.contains(native) && found.contains(native));
            if is_live(chat) {
                prop_assert!(copied != native, "{:?} comes over exactly one way", chat);
            } else {
                prop_assert!(!copied, "{:?} never comes over", chat);
            }
        }
        let links = links(&store);
        for (worker, lead) in &links.chats {
            prop_assert_ne!(worker, lead, "no chat leads itself");
            prop_assert!(links.coordinators.contains(lead));
        }
        let candidates = worktree_candidates(&store);
        let paths: HashSet<&PathBuf> = candidates.iter().map(|candidate| &candidate.path).collect();
        prop_assert_eq!(paths.len(), candidates.len());
    }

    /// Worktree ids never panic, and `<plain id>::<absolute path>` round-trips.
    #[test]
    fn worktree_ids_never_panic_and_round_trip(
        raw in "[ -~]{0,40}",
        repo in "[A-Za-z0-9_-]{1,20}",
        path in "(/[A-Za-z0-9 ._-]{1,10}){1,4}",
        suffix in proptest::option::of("[a-f0-9-]{1,12}"),
    ) {
        let _ = store::place_of(&raw);
        let id = match &suffix {
            Some(suffix) => format!("{repo}::{path}::workspace:{suffix}"),
            None => format!("{repo}::{path}"),
        };
        prop_assert_eq!(store::place_of(&id), Place::Worktree { repo, path: PathBuf::from(path) });
    }

    /// `orca.yaml` never panics, and single-line scripts round-trip.
    #[test]
    fn orca_yaml_never_panics_and_plain_scripts_round_trip(
        raw in "[ -~\n]{0,200}",
        setup in "[A-Za-z0-9 ./_-]{1,30}",
        archive in proptest::option::of("[A-Za-z0-9 ./_-]{1,30}"),
    ) {
        let _ = store::yaml_scripts(&raw);
        let mut text = format!("version: 1\nscripts:\n  setup: {setup}\n");
        if let Some(archive) = &archive {
            text.push_str(&format!("  archive: \"{archive}\"\n"));
        }
        text.push_str("issueCommand: x\n");
        let scripts = store::yaml_scripts(&text);
        let want = Some(setup.trim().to_owned()).filter(|setup| !setup.is_empty());
        let want_archive = archive.as_deref().map(str::trim).filter(|archive| !archive.is_empty()).map(str::to_owned);
        prop_assert_eq!(scripts.as_ref().and_then(|scripts| scripts.setup.clone()), want);
        prop_assert_eq!(scripts.and_then(|scripts| scripts.archive), want_archive);
    }

    /// A journal payload of any shape never panics, and only conversation
    /// becomes an entry.
    #[test]
    fn any_journal_payload_reads_safely(payload in any_json()) {
        if let Some(item) = store::item_of(&payload) {
            let kind = payload.pointer("/body/kind").and_then(Value::as_str);
            prop_assert!(matches!(kind, Some("message" | "tool-call")));
            prop_assert!(payload.get("agentId").is_none_or(Value::is_null));
            if let Body::User(text) | Body::Assistant(text) | Body::Reasoning(text) = &item.body {
                prop_assert!(!text.trim().is_empty());
            }
        }
    }
}

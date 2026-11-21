use super::fixture::{Fixture, ThreadRow, CANARY, VERSIONS};
use super::store::{self, Era, NativeSession, Role};
use super::*;
use crate::import_sqlite::STATEMENTS;
use serde_json::json;

fn row<'a>(id: &'a str, title: &'a str, native: Option<&'a str>) -> ThreadRow<'a> {
    ThreadRow { id, project: "p", title, driver: "claudeAgent", native, updated_at: "2026-11-05T00:00:00.000Z", ..ThreadRow::default() }
}

fn read_fixture(fixture: &Fixture) -> Store {
    store::read(&fixture.conn).unwrap()
}

fn claude(id: &str) -> NativeSession {
    NativeSession { harness: "claude", id: id.into() }
}

#[test]
fn every_real_schema_reads_projects_threads_worktrees_and_native_sessions() {
    for &version in VERSIONS {
        let dir = tempfile::tempdir().unwrap();
        let fixture = Fixture::in_base(dir.path(), version);
        fixture.project("p", Path::new("/code/shop"), "Shop");
        let worktree = Path::new("/home/.t3/worktrees/shop/feat-a-0123");
        fixture.thread(&ThreadRow { branch: Some("feat/a"), worktree: Some(worktree), ..row("t1", "Add a discount", Some("11111111-2222-4333-8444-555555555555")) });
        fixture.thread(&ThreadRow { driver: "codex", archived: true, ..row("t2", "Round prices", Some("01a1f9f8-cbdc-7a20-a62a-3e214132a143")) });
        let store = read_fixture(&fixture);
        assert_eq!(store.version, version);
        assert_eq!(store.era, if version >= 55 { Era::V2 } else { Era::V1 }, "{version}");
        assert_eq!(store.projects["p"].root, PathBuf::from("/code/shop"));
        let by = |id: &str| store.threads.iter().find(|thread| thread.id == id).unwrap().clone();
        let first = by("t1");
        assert_eq!(first.title.as_deref(), Some("Add a discount"), "{version}");
        assert_eq!(first.worktree.as_deref(), Some(worktree), "{version}");
        assert_eq!(first.branch.as_deref(), Some("feat/a"), "{version}");
        assert_eq!(first.native, Some(claude("11111111-2222-4333-8444-555555555555")), "{version}");
        let second = by("t2");
        assert_eq!(second.native, Some(NativeSession { harness: "codex", id: "01a1f9f8-cbdc-7a20-a62a-3e214132a143".into() }), "{version}");
        // Archiving arrived in migration 17.
        assert_eq!(second.archived, version >= 17, "{version}");
        assert!(!first.archived && !first.deleted);
        assert_eq!(chats(&store).count(), 2);
    }
}

#[test]
fn scripts_split_into_setup_and_run() {
    let scripts = store::scripts_of(&json!([
        {"name": "Install", "command": "bun install", "runOnWorktreeCreate": true},
        {"name": "Env", "command": "cp .env.example .env", "runOnWorktreeCreate": true},
        {"name": "Dev", "command": "bun dev"},
        {"name": "Test", "command": "bun test"},
    ]))
    .unwrap();
    assert_eq!(scripts.setup.as_deref(), Some("bun install\ncp .env.example .env"));
    assert_eq!(scripts.run.as_deref(), Some("bun dev"));
    assert!(store::scripts_of(&json!([])).is_none());
    assert!(store::scripts_of(&json!([{"name": "blank", "command": "  "}])).is_none());
    assert!(store::scripts_of(&json!({"not": "a list"})).is_none());
}

#[test]
fn cursors_name_the_native_session_per_driver() {
    let claude_cursor = json!({"threadId": "t3-thread", "resume": "abc-1", "resumeSessionAt": "m"});
    assert_eq!(store::native_from_cursor("claudeAgent", &claude_cursor), Some(claude("abc-1")));
    assert_eq!(store::native_from_cursor("claude", &json!({"sessionId": "abc-2"})), Some(claude("abc-2")), "older builds");
    assert_eq!(store::native_from_cursor("claudeAgent", &json!({"threadId": "t3-thread"})), None, "Claude's threadId is T3 Code's own id");
    assert_eq!(
        store::native_from_cursor("codex", &json!({"threadId": "cx-1"})),
        Some(NativeSession { harness: "codex", id: "cx-1".into() })
    );
    assert_eq!(store::native_from_cursor("cursor", &json!({"threadId": "cu-1"})), None, "no transcript Bridge reads");
    for bad in ["", "  ", "../etc", "a/b", "a b", "x:y", "x#y", "."] {
        assert_eq!(store::native_from_cursor("codex", &json!({"threadId": bad})), None, "{bad:?}");
    }
    assert_eq!(store::native_from_ref(None, &json!({"nativeThreadRef": {"driver": "opencode", "nativeId": "ses_1"}})).map(|n| n.harness), Some("opencode"));
    assert_eq!(store::native_from_ref(Some("codex"), &json!({"nativeThreadRef": {"nativeId": "cx-2"}})).map(|n| n.harness), Some("codex"));
    assert_eq!(store::native_from_ref(Some("grok"), &json!({"nativeThreadRef": {"nativeId": "g"}})), None);
}

#[test]
fn deleted_threads_and_threads_of_deleted_projects_are_left_out() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::in_base(dir.path(), 102);
    fixture.project("p", Path::new("/code/a"), "A");
    fixture.project("gone", Path::new("/code/b"), "B");
    fixture.thread(&row("live", "Live", None));
    fixture.thread(&ThreadRow { deleted: true, ..row("deleted", "Deleted", None) });
    fixture.thread(&ThreadRow { project: "gone", ..row("orphaned", "In a deleted project", None) });
    fixture.thread(&ThreadRow { project: "missing", ..row("nowhere", "No project row", None) });
    fixture.delete_project("gone");
    let store = read_fixture(&fixture);
    assert_eq!(chats(&store).map(|thread| thread.id.as_str()).collect::<Vec<_>>(), vec!["live"]);
}

#[test]
fn a_subagent_comes_under_its_parent_and_is_not_counted() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::in_base(dir.path(), 102);
    fixture.project("p", Path::new("/code/a"), "A");
    fixture.thread(&row("parent", "Explain pricing", Some("c8f56b8b-b26c-4398-9f58-93fdb97b388a")));
    fixture.thread(&ThreadRow { parent: Some("parent"), ..row("thread:subagent:11c39a8cf86c5ec5", "Find price.js", None) });
    let store = read_fixture(&fixture);
    assert_eq!(chats(&store).count(), 1);
    let child = store.threads.iter().find(|thread| thread.parent.is_some()).unwrap();
    assert_eq!(child.parent.as_deref(), Some("parent"));
    assert_eq!(counts(&store).chats, 1);
    // Natively read Claude parent: the subagent comes in its transcript.
    let plan = plan_chats(&store, &HashSet::from([claude("c8f56b8b-b26c-4398-9f58-93fdb97b388a")]));
    assert_eq!(plan.native.len(), 1);
    assert!(plan.copies.is_empty());
    // Parent's transcript gone: both are T3 Code's copies.
    let plan = plan_chats(&store, &HashSet::new());
    assert_eq!(plan.copies.len(), 2);
}

#[test]
fn the_active_provider_thread_wins_else_the_latest() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::in_base(dir.path(), 102);
    fixture.project("p", Path::new("/code/a"), "A");
    fixture.thread(&row("t", "Handed off", Some("new-session")));
    fixture.provider_thread("old", "t", "claudeAgent", "old-session", "2026-12-01T00:00:00.000Z");
    let store = read_fixture(&fixture);
    assert_eq!(store.threads[0].native, Some(claude("new-session")), "active beats newer");
    fixture.conn.execute("UPDATE orchestration_v2_projection_threads SET active_provider_thread_id=NULL", []).unwrap();
    fixture.provider_thread("older", "t", "claudeAgent", "oldest-session", "2026-10-01T00:00:00.000Z");
    let store = read_fixture(&fixture);
    assert_eq!(store.threads[0].native, Some(claude("old-session")), "then the one used last");
}

#[test]
fn an_active_provider_thread_of_another_thread_is_not_used() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::in_base(dir.path(), 102);
    fixture.project("p", Path::new("/code/a"), "A");
    fixture.thread(&row("a", "A", Some("session-a")));
    fixture.thread(&row("b", "B", None));
    fixture
        .conn
        .execute(
            "UPDATE orchestration_v2_projection_threads SET active_provider_thread_id=(SELECT active_provider_thread_id FROM orchestration_v2_projection_threads WHERE thread_id='a') WHERE thread_id='b'",
            [],
        )
        .unwrap();
    let store = read_fixture(&fixture);
    assert_eq!(store.threads.iter().find(|thread| thread.id == "b").unwrap().native, None);
}

#[test]
fn an_upgraded_store_finds_old_sessions_in_cursors_and_untranscribed_messages_in_v1() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::in_base(dir.path(), 102);
    fixture.project("p", Path::new("/code/a"), "A");
    fixture.upgraded_thread(&row("old", "Casual conversation", Some("43440686-a49e-487a-9544-a94e260774a6")), false);
    fixture.upgraded_thread(&row("copied", "hi", Some("bbf9b342-fc45-4176-84ca-29946944abb9")), true);
    fixture.legacy_message("old", "m1", "user", "hi", "2026-09-07T20:35:00.000Z", false);
    fixture.legacy_message("old", "m2", "assistant", "Hi! What are we working on?", "2026-09-07T20:35:01.000Z", false);
    fixture.message("copied", "n1", "user", "hello", "2026-09-23T07:34:00.000Z", false);
    let store = read_fixture(&fixture);
    let old = store.threads.iter().find(|thread| thread.id == "old").unwrap();
    assert_eq!(old.native, Some(claude("43440686-a49e-487a-9544-a94e260774a6")));
    assert!(old.legacy_messages);
    let texts: Vec<String> = store::messages(&fixture.conn, store.era, old).unwrap().into_iter().map(|message| message.text).collect();
    assert_eq!(texts, vec!["hi", "Hi! What are we working on?"]);
    let copied = store.threads.iter().find(|thread| thread.id == "copied").unwrap();
    assert!(!copied.legacy_messages);
    assert_eq!(store::messages(&fixture.conn, store.era, copied).unwrap().len(), 1);
}

#[test]
fn the_copy_keeps_user_and_assistant_text_in_order() {
    for version in [43, 102] {
        let dir = tempfile::tempdir().unwrap();
        let fixture = Fixture::in_base(dir.path(), version);
        fixture.project("p", Path::new("/code/a"), "A");
        fixture.thread(&row("t", "Order", Some("s-1")));
        fixture.message("t", "b", "assistant", "second", "2026-11-05T00:00:02.000Z", false);
        fixture.message("t", "a", "user", "first", "2026-11-05T00:00:01.000Z", false);
        fixture.message("t", "c", "system", "a system note", "2026-11-05T00:00:03.000Z", false);
        fixture.message("t", "d", "assistant", "   ", "2026-11-05T00:00:04.000Z", false);
        fixture.message("t", "e", "assistant", "half a reply", "2026-11-05T00:00:05.000Z", true);
        fixture.message("t", "f", "user", "  spaced  ", "2026-11-05T00:00:06.000Z", false);
        let store = read_fixture(&fixture);
        let messages = store::messages(&fixture.conn, store.era, &store.threads[0]).unwrap();
        assert_eq!(
            messages.iter().map(|message| (message.role, message.text.as_str())).collect::<Vec<_>>(),
            vec![(Role::User, "first"), (Role::Assistant, "second"), (Role::User, "  spaced  ")],
            "{version}"
        );
        let chat = copy_chat(&store, &store.threads[0], &messages, "ref");
        assert_eq!(chat.entries_source, crate::import_bundle::EntriesSource::ToolCopy);
        assert_eq!(chat.native_ref.as_ref().map(NativeRef::key).as_deref(), Some("claude:s-1"));
        assert_eq!(chat.title.as_deref(), Some("Order"));
        assert_eq!(chat.cwd.as_deref(), Some("/code/a"));
        assert_eq!(chat.entries.len(), 3);
    }
}

#[test]
fn the_version_gate_fails_closed_with_the_version_it_saw() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::in_base(dir.path(), 102);
    fixture.conn.execute("INSERT INTO effect_sql_migrations(migration_id,name) VALUES(103,'Future')", []).unwrap();
    let error = store::read(&fixture.conn).unwrap_err();
    assert!(error.contains("newer") && error.contains("103"), "{error}");

    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::in_base(dir.path(), 5);
    fixture.conn.execute("DELETE FROM effect_sql_migrations WHERE migration_id>=5", []).unwrap();
    let error = store::read(&fixture.conn).unwrap_err();
    assert!(error.contains("older") && error.contains('4'), "{error}");

    fixture.conn.execute("DELETE FROM effect_sql_migrations", []).unwrap();
    assert!(store::read(&fixture.conn).unwrap_err().contains("no version"));

    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::in_base(dir.path(), 102);
    fixture.conn.execute_batch("ALTER TABLE orchestration_v2_projection_threads RENAME COLUMN payload_json TO payload").unwrap();
    assert!(store::read(&fixture.conn).unwrap_err().contains("missing what Bridge reads (version 102)"));

    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::in_base(dir.path(), 43);
    fixture.conn.execute_batch("ALTER TABLE projection_projects RENAME COLUMN workspace_root TO root").unwrap();
    assert!(store::read(&fixture.conn).unwrap_err().contains("missing what Bridge reads (version 43)"));
}

#[test]
fn a_file_that_is_not_a_database_or_is_empty_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::at(dir.path());
    std::fs::create_dir_all(paths.store_v1.parent().unwrap()).unwrap();
    std::fs::write(&paths.store_v1, "{}").unwrap();
    assert!(read(&paths).unwrap_err().contains("T3 Code's database"));
    std::fs::write(&paths.store_v1, "").unwrap();
    assert!(read(&paths).is_err());
}

#[test]
fn the_v2_file_wins_over_the_old_one() {
    let dir = tempfile::tempdir().unwrap();
    let old = Fixture::in_base(dir.path(), 43);
    old.project("p", Path::new("/code/a"), "A");
    old.thread(&row("from-v1", "Only in the old file", None));
    let paths = Paths::at(dir.path());
    assert_eq!(paths.store(), Some(paths.store_v1.as_path()));
    let new = Fixture::in_base(dir.path(), 102);
    new.project("p", Path::new("/code/a"), "A");
    new.thread(&row("from-v2", "In the new file", None));
    assert_eq!(paths.store(), Some(paths.store_v2.as_path()));
    let (store, path) = read(&paths).unwrap();
    assert_eq!(path, paths.store_v2);
    assert_eq!(store.threads.iter().map(|thread| thread.id.as_str()).collect::<Vec<_>>(), vec!["from-v2"]);
}

#[test]
fn t3code_home_moves_everything() {
    let paths = Paths::for_home(Path::new("/Users/me"), None);
    assert_eq!(paths.base, PathBuf::from("/Users/me/.t3"));
    assert_eq!(paths.store_v2, PathBuf::from("/Users/me/.t3/userdata/statev2.sqlite"));
    let paths = Paths::for_home(Path::new("/Users/me"), Some(Path::new("/elsewhere/t3")));
    assert_eq!(paths.store_v1, PathBuf::from("/elsewhere/t3/userdata/state.sqlite"));
    assert!(!paths.installed());
}

#[test]
fn only_allowed_tables_are_queried_and_no_secret_reaches_the_output() {
    for &version in VERSIONS {
        let dir = tempfile::tempdir().unwrap();
        let fixture = Fixture::in_base(dir.path(), version);
        fixture.project("p", Path::new("/code/a"), "A");
        fixture.thread(&row("t", "Thread", Some("s-1")));
        fixture.message("t", "m", "user", "hello", "2026-11-05T00:00:01.000Z", false);
        STATEMENTS.with(|statements| statements.borrow_mut().clear());
        let store = read_fixture(&fixture);
        let messages = store::messages(&fixture.conn, store.era, &store.threads[0]).unwrap();
        let chat = copy_chat(&store, &store.threads[0], &messages, "ref");
        let statements = STATEMENTS.with(|statements| statements.borrow().clone());
        assert!(!statements.is_empty());
        for statement in &statements {
            assert!(
                store::ALLOWED_TABLES.iter().any(|table| statement.contains(&format!(" {table}")) || statement.contains(&format!("'{table}'"))),
                "{statement}"
            );
            for denied in store::DENIED_TABLES {
                assert!(!statement.contains(denied), "{statement} names {denied}");
            }
            assert!(!statement.contains("runtime_payload_json"), "{statement}");
        }
        let output = format!("{store:?}{messages:?}{}", serde_json::to_string(&chat).unwrap());
        assert!(!output.contains(CANARY), "{version}: a secret reached the output");
        // The canaries are really there.
        if fixture.has("auth_sessions", "session_id") {
            let found: i64 = fixture.conn.query_row("SELECT count(*) FROM auth_sessions", [], |row| row.get(0)).unwrap();
            assert_eq!(found, 1);
        }
    }
}

#[test]
fn a_closed_database_is_read_without_creating_a_file_and_a_live_wal_is_read_through() {
    let dir = tempfile::tempdir().unwrap();
    let path = {
        let fixture = Fixture::in_base(dir.path(), 102);
        fixture.project("p", Path::new("/code/a"), "A");
        fixture.thread(&row("t", "Thread", None));
        fixture.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;").unwrap();
        fixture.path.clone()
    };
    let before: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().map(|entry| entry.unwrap().file_name()).collect();
    let (store, _) = read(&Paths::at(dir.path())).unwrap();
    assert_eq!(store.threads.len(), 1);
    let after: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().map(|entry| entry.unwrap().file_name()).collect();
    assert_eq!(before, after, "no -wal or -shm created");

    let live = tempfile::tempdir().unwrap();
    let fixture = Fixture::in_base(live.path(), 102);
    fixture.project("p", Path::new("/code/a"), "A");
    fixture.thread(&row("t", "Not checkpointed yet", None));
    assert!(std::fs::metadata(format!("{}-wal", fixture.path.display())).unwrap().len() > 0);
    let (store, _) = read(&Paths::at(live.path())).unwrap();
    assert_eq!(store.threads[0].title.as_deref(), Some("Not checkpointed yet"));
}

#[test]
fn worktrees_are_candidates_once_named_for_their_newest_thread_and_archived_when_all_are() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::in_base(dir.path(), 102);
    fixture.project("p", Path::new("/code/shop"), "Shop");
    let shared = Path::new("/w/shop/feat-a");
    let parked = Path::new("/w/shop/old");
    fixture.thread(&ThreadRow { worktree: Some(shared), updated_at: "2026-11-01T00:00:00.000Z", ..row("a", "Older", None) });
    fixture.thread(&ThreadRow { worktree: Some(shared), updated_at: "2026-11-09T00:00:00.000Z", archived: true, ..row("b", "Newer", None) });
    fixture.thread(&ThreadRow { worktree: Some(parked), archived: true, ..row("c", "Parked", None) });
    fixture.thread(&ThreadRow { worktree: Some(Path::new("relative/path")), ..row("d", "Relative", None) });
    fixture.thread(&ThreadRow { worktree: Some(Path::new("/w/shop/deleted")), deleted: true, ..row("e", "Deleted", None) });
    fixture.thread(&row("f", "No worktree", None));
    let store = read_fixture(&fixture);
    let candidates = worktree_candidates(&store);
    assert_eq!(candidates.iter().map(|candidate| candidate.path.as_path()).collect::<Vec<_>>(), vec![shared, parked]);
    assert_eq!(candidates[0].title.as_deref(), Some("Newer"));
    assert!(!candidates[0].archived, "one live thread keeps it");
    assert!(candidates[1].archived);
    assert_eq!(candidates[0].scripts.as_ref().and_then(|scripts| scripts.setup.as_deref()), Some("bun install"));
    assert_eq!(candidates[0].tool, PROVIDER);
    assert_eq!(counts(&store), Counts { chats: 5, projects: 1, workspaces: 1 });
}

#[test]
fn an_archived_project_archives_its_threads() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::in_base(dir.path(), 102);
    fixture.project("p", Path::new("/code/shop"), "Shop");
    fixture.thread(&row("a", "In an archived project", None));
    fixture.conn.execute("UPDATE projection_projects SET archived_at='2026-11-04T00:00:00.000Z'", []).unwrap();
    let store = read_fixture(&fixture);
    assert!(is_archived(&store, &store.threads[0]));
    assert!(copy_chat(&store, &store.threads[0], &[], "ref").archived);
}

#[test]
fn source_refs_round_trip() {
    let reference = source_ref(Path::new("/Users/me/.t3/userdata/statev2.sqlite"), "mcp:f1cf9272");
    assert_eq!(parse_source_ref(&reference), Some((PathBuf::from("/Users/me/.t3/userdata/statev2.sqlite"), "mcp:f1cf9272".into())));
    assert_eq!(parse_source_ref("no-hash"), None);
    assert_eq!(parse_source_ref("#t"), None);
    assert_eq!(parse_source_ref("/s#"), None);
}

#[test]
fn the_pin_and_pull_request_links_are_read() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::in_base(dir.path(), 102);
    fixture.project("p", Path::new("/code/shop"), "Shop");
    fixture.thread(&ThreadRow { pinned: true, ..row("a", "Pinned", None) });
    fixture
        .conn
        .execute(
            "UPDATE orchestration_v2_projection_threads SET payload_json=json_set(payload_json,'$.pullRequests',json('[{\"number\":1},{\"number\":2}]'))",
            [],
        )
        .unwrap();
    let store = read_fixture(&fixture);
    assert!(store.threads[0].pinned);
    assert_eq!(store.threads[0].pull_requests, 2);
}

#[test]
fn a_corrupt_payload_reads_as_empty_fields_not_a_failure() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::in_base(dir.path(), 102);
    fixture.project("p", Path::new("/code/shop"), "Shop");
    fixture.thread(&ThreadRow { worktree: Some(Path::new("/w/x")), ..row("a", "Broken", Some("s")) });
    fixture.conn.execute("UPDATE orchestration_v2_projection_threads SET payload_json='not json'", []).unwrap();
    fixture.conn.execute("UPDATE orchestration_v2_projection_provider_threads SET payload_json='{'", []).unwrap();
    let store = read_fixture(&fixture);
    assert_eq!(store.threads[0].title.as_deref(), Some("Broken"));
    assert_eq!(store.threads[0].worktree, None);
    assert_eq!(store.threads[0].native, None);
}


#[test]
fn a_copy_without_a_native_session_takes_its_harness_from_the_driver() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::in_base(dir.path(), 102);
    fixture.project("p", Path::new("/code/a"), "A");
    fixture.thread(&ThreadRow { driver: "codex", ..row("c", "Codex, no session", None) });
    fixture.thread(&ThreadRow { driver: "cursor", ..row("u", "Cursor", Some("cursor-session")) });
    let store = read_fixture(&fixture);
    let by = |id: &str| store.threads.iter().find(|thread| thread.id == id).unwrap();
    assert_eq!(copy_chat(&store, by("c"), &[], "r").harness, "codex");
    let cursor = copy_chat(&store, by("u"), &[], "r");
    assert_eq!((cursor.harness.as_str(), cursor.native_ref.is_none()), ("other", true), "no transcript Bridge reads");
}

#[test]
fn ids_are_plain_and_bounded() {
    let id = |text: &str| store::native_from_cursor("codex", &json!({ "threadId": text })).map(|native| native.id);
    assert_eq!(id(&"a".repeat(200)).map(|id| id.len()), Some(200));
    assert_eq!(id(&"a".repeat(201)), None);
    assert_eq!(id(".."), None);
    assert_eq!(id("a\\b"), None);
    assert_eq!(id("..x").as_deref(), Some("..x"));
    assert_eq!(id("  padded  ").as_deref(), Some("padded"));
}

#[test]
fn a_linked_pull_request_alone_counts_once() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::in_base(dir.path(), 102);
    fixture.project("p", Path::new("/code/a"), "A");
    fixture.thread(&row("a", "Linked", None));
    fixture
        .conn
        .execute(
            "UPDATE orchestration_v2_projection_threads SET payload_json=json_set(payload_json,'$.linkedPullRequest',json('{\"number\":3}'))",
            [],
        )
        .unwrap();
    assert_eq!(read_fixture(&fixture).threads[0].pull_requests, 1);
}

#[test]
fn only_t3_codes_own_safe_candidates_are_normalized() {
    use crate::external_import::{DiscoverySelection, ExternalHarnessImporter};
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    {
        let fixture = Fixture::in_base(&base, 102);
        fixture.project("p", Path::new("/code/a"), "A");
        fixture.thread(&row("t", "Thread", Some("s-1")));
        fixture.message("t", "m", "user", "hello", "2026-11-05T00:00:01.000Z", false);
    }
    let (store, path) = read(&Paths::at(&base)).unwrap();
    let artifacts = vec![copy_artifact(&path, &store, &store.threads[0])];
    let selection = DiscoverySelection { artifact_ids: artifacts.iter().map(|artifact| artifact.artifact_id.clone()).collect() };
    let discovery = job_discovery(std::slice::from_ref(&base), artifacts);
    let candidates = T3CodeImporter.preview(&discovery, &selection).unwrap();
    assert_eq!(candidates.len(), 1);
    let candidate = candidates[0].clone();
    let normalized = T3CodeImporter.normalize(candidate.clone()).unwrap();
    assert!(normalized.deterministic_bridge_id.starts_with("imp_"), "{}", normalized.deterministic_bridge_id);
    assert_eq!(normalized.deterministic_bridge_id, T3CodeImporter.normalize(candidate.clone()).unwrap().deterministic_bridge_id, "stable");
    let mut other = candidate.clone();
    other.source.provider = "claude_code".into();
    assert!(T3CodeImporter.normalize(other).is_err());
    let mut unsafe_one = candidate;
    unsafe_one.redaction_summary.safely_representable = false;
    assert!(T3CodeImporter.normalize(unsafe_one).is_err());

    let mut wrong = discovery.clone();
    wrong.provider = "codex".into();
    assert!(T3CodeImporter.preview(&wrong, &selection).is_err());
    let mut ungated = discovery.clone();
    ungated.format_versions.clear();
    assert!(T3CodeImporter.preview(&ungated, &selection).is_err());
    let outside = job_discovery(&[PathBuf::from("/nowhere")], discovery.artifacts.clone());
    let refused = T3CodeImporter.preview(&outside, &selection).unwrap();
    assert_eq!(refused[0].kind, crate::external_import::CandidateKind::Unsupported, "a database outside the approved folders is refused");
    assert!(T3CodeImporter.preview(&discovery, &DiscoverySelection { artifact_ids: Vec::new() }).unwrap().is_empty());
}

#[test]
fn copied_subagents_link_to_their_parent_by_key_or_by_native_identity() {
    use crate::external_import::{DiscoverySelection, ExternalHarnessImporter};
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    {
        let fixture = Fixture::in_base(&base, 102);
        fixture.project("p", Path::new("/code/a"), "A");
        fixture.thread(&ThreadRow { driver: "codex", ..row("parent", "Parent", Some("cx-parent")) });
        fixture.thread(&ThreadRow { driver: "codex", parent: Some("parent"), ..row("child", "Child", Some("cx-child")) });
        for (thread, text) in [("parent", "plan it"), ("child", "look around")] {
            fixture.message(thread, &format!("{thread}-m"), "user", text, "2026-11-05T00:00:01.000Z", false);
        }
    }
    let (store, path) = read(&Paths::at(&base)).unwrap();
    let artifact = |id: &str| copy_artifact(&path, &store, store.threads.iter().find(|thread| thread.id == id).unwrap());
    let preview = |ids: &[&str]| {
        let artifacts: Vec<_> = ids.iter().map(|id| artifact(id)).collect();
        let selection = DiscoverySelection { artifact_ids: artifacts.iter().map(|artifact| artifact.artifact_id.clone()).collect() };
        T3CodeImporter.preview(&job_discovery(std::slice::from_ref(&base), artifacts), &selection).unwrap()
    };
    let payload = |candidates: &[crate::external_import::ImportCandidate], title: &str| {
        candidates.iter().find(|candidate| candidate.title == title).unwrap().normalized_payload.clone()
    };
    // Together: the child points at the parent's candidate.
    let together = preview(&["parent", "child"]);
    let parent_id = together.iter().find(|candidate| candidate.title == "Parent").unwrap().candidate_id.clone();
    assert_eq!(payload(&together, "Child")["parentCandidateId"].as_str(), Some(parent_id.as_str()));
    // Alone: it points at the parent's native session, for the writer to find.
    let alone = preview(&["child"]);
    assert_eq!(payload(&alone, "Child")["parentNativeKey"].as_str(), Some("codex:cx-parent"));
    assert_eq!(payload(&alone, "Child")["agentRole"].as_str(), Some("subagent"));
}

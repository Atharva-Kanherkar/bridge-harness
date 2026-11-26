use super::fixture::{message, status, subagent, tool, turn, worktree_id, ChatRow, Fixture, Handle, CANARY};
use super::store::{self, Body, NativeSession, Place, Worker};
use super::*;
use crate::import_sqlite::STATEMENTS;
use serde_json::json;

const REPO: &str = "6e1ad727-cfd9-4417-b1dc-bdcb536e895f";

fn opencode(id: &str) -> NativeSession {
    NativeSession { harness: "opencode", id: id.into() }
}

fn read_base(base: &Path) -> Store {
    read(&Paths::at(base)).unwrap().0
}

/// A data folder like the sandbox's: a repository with two worktrees, an
/// OpenCode chat in one renamed in Orca, a coordinator chat in the main
/// checkout, and a Claude Code chat that never started.
struct Sandbox {
    _dir: tempfile::TempDir,
    base: PathBuf,
    main: PathBuf,
    login: PathBuf,
    contrib: PathBuf,
    fixture: Fixture,
}

fn sandbox() -> Sandbox {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("orca");
    let main = dir.path().join("code/shop");
    let login = dir.path().join("orca/workspaces/shop/feat-login");
    let contrib = dir.path().join("orca/workspaces/shop/contrib");
    let fixture = Fixture::at(&base);
    fixture.repos("local-default", &[(REPO, &main)]);
    fixture.worktree_meta(
        "local-default",
        &[(worktree_id(REPO, &main), "", false), (worktree_id(REPO, &login), "Login page", false), (worktree_id(REPO, &contrib), "contrib", false)],
    );
    fixture.chat(&ChatRow {
        id: "16acf9ae",
        agent: "opencode",
        workspace: worktree_id(REPO, &login),
        title: Some("What is shop?"),
        renamed: true,
        handles: vec![Handle::Acp("opencode", "ses_a")],
        ..ChatRow::default()
    });
    fixture.chat(&ChatRow {
        id: "35dcaa0e",
        agent: "opencode",
        workspace: worktree_id(REPO, &main),
        title: Some("Orca LICENSE Orchestration"),
        handles: vec![Handle::Acp("opencode", "ses_b")],
        ..ChatRow::default()
    });
    fixture.chat(&ChatRow { id: "f441372b", agent: "claude", workspace: worktree_id(REPO, &contrib), title: Some("Say hi"), ..ChatRow::default() });
    Sandbox { _dir: dir, base, main, login, contrib, fixture }
}

#[test]
fn the_real_schema_reads_repositories_worktrees_and_chats() {
    let sandbox = sandbox();
    let store = read_base(&sandbox.base);
    assert_eq!(store.chat_version, 2);
    assert_eq!(store.profiles.len(), 1);
    let profile = &store.profiles[0];
    assert_eq!(profile.repos, vec![store::Repo { id: REPO.into(), path: sandbox.main.clone(), name: Some("shop".into()), remote: false }]);
    let names: Vec<(PathBuf, Option<String>)> = profile.worktrees.iter().map(|worktree| (worktree.path.clone(), worktree.name.clone())).collect();
    assert!(names.contains(&(sandbox.login.clone(), Some("Login page".into()))), "{names:?}");
    let by = |id: &str| store.chats.iter().find(|chat| chat.id == id).unwrap().clone();
    let first = by("16acf9ae");
    assert_eq!(first.native, Some(opencode("ses_a")));
    assert_eq!(first.title.as_deref(), Some("What is shop?"));
    assert!(first.renamed && first.local);
    assert_eq!(first.place, Place::Worktree { repo: REPO.into(), path: sandbox.login.clone() });
    assert_eq!(first.created_at.as_deref(), Some("2026-11-23T07:32:38.665+00:00"));
    assert_eq!(folder(&store, &first), Some(sandbox.login.clone()));
    let never_started = by("f441372b");
    assert_eq!(never_started.native, None);
    assert!(!never_started.renamed);
    assert_eq!(chats(&store).count(), 3);
    assert_eq!(counts(&store), Counts { chats: 3, projects: 1, workspaces: 2 });
    assert!(store.notes.is_empty(), "every profile was read: {:?}", store.notes);
}

#[test]
fn every_handle_form_names_its_native_session_and_the_last_link_wins() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::at(dir.path());
    let chat = |id, agent, handles| fixture.chat(&ChatRow { id, agent, workspace: "global-floating-terminal".into(), handles, ..ChatRow::default() });
    chat("c1", "claude", vec![Handle::Claude("old"), Handle::Claude("11111111-1111-4111-8111-111111111111")]);
    chat("c2", "codex", vec![Handle::Codex("01a1f9f8-cbdc-7a20-a62a-3e214132a143")]);
    chat("c3", "opencode", vec![Handle::Acp("opencode", "ses_x")]);
    chat("c4", "pi", vec![Handle::Acp("pi", "pi-1")]);
    chat("c5", "gemini", vec![Handle::Acp("gemini", "g-1")]);
    let store = read_base(dir.path());
    let native = |id: &str| store.chats.iter().find(|chat| chat.id == id).unwrap().native.clone();
    assert_eq!(native("c1"), Some(NativeSession { harness: "claude", id: "11111111-1111-4111-8111-111111111111".into() }));
    assert_eq!(native("c2"), Some(NativeSession { harness: "codex", id: "01a1f9f8-cbdc-7a20-a62a-3e214132a143".into() }));
    assert_eq!(native("c3"), Some(opencode("ses_x")));
    assert_eq!(native("c4"), None, "Bridge does not run Pi");
    assert_eq!(native("c5"), None);
    let live: Vec<&str> = chats(&store).map(|chat| chat.id.as_str()).collect();
    assert_eq!(live, vec!["c1", "c2", "c3"], "agents Bridge does not run are left out");
    assert_eq!(store::native_of(Some("claude"), None, None, Some("t"), None), None, "Claude Code's id is its session id");
    assert_eq!(store::native_of(Some("codex"), None, Some("s"), None, None), None, "Codex's id is its thread id");
    assert_eq!(store::native_of(Some("cursor"), None, Some("s"), Some("t"), Some("n")), None);
    assert_eq!(store::native_of(None, Some("opencode"), None, None, Some("a/b")), None, "ids are plain");
}

#[test]
fn worktree_ids_read_their_path() {
    let worktree = |repo: &str, path: &str| Place::Worktree { repo: repo.into(), path: PathBuf::from(path) };
    assert_eq!(store::place_of("r1::/a/b"), worktree("r1", "/a/b"));
    assert_eq!(store::place_of("r1::/a/b::workspace:0b2f"), worktree("r1", "/a/b"));
    assert_eq!(store::place_of("r1::/a::b/c"), worktree("r1", "/a::b/c"), "a path may hold ::");
    assert_eq!(store::place_of("r1::/a/b::workspace:"), worktree("r1", "/a/b::workspace:"), "an empty suffix is part of the path");
    assert_eq!(store::place_of("folder:f1"), Place::Folder("f1".into()));
    assert_eq!(store::place_of("folder:"), Place::None);
    assert_eq!(store::place_of("global-floating-terminal"), Place::None);
    assert_eq!(store::place_of("r1::relative/path"), Place::None);
    assert_eq!(store::place_of("::/a"), Place::None);
    assert_eq!(store::place_of("r/1::/a"), Place::None);
    assert_eq!(store::place_of(""), Place::None);
}

#[test]
fn a_folder_workspace_or_launch_directory_is_the_chats_folder() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::at(dir.path());
    fixture.folder_workspaces("local-default", &[("f1", Path::new("/notes"))]);
    fixture.chat(&ChatRow { id: "a", workspace: "folder:f1".into(), ..ChatRow::default() });
    fixture.chat(&ChatRow { id: "b", workspace: "global-floating-terminal".into(), launch: Some(Path::new("/tmp/x")), ..ChatRow::default() });
    fixture.chat(&ChatRow { id: "c", workspace: "folder:gone".into(), ..ChatRow::default() });
    let store = read_base(dir.path());
    let folder_of = |id: &str| folder(&store, store.chats.iter().find(|chat| chat.id == id).unwrap());
    assert_eq!(folder_of("a"), Some(PathBuf::from("/notes")));
    assert_eq!(folder_of("b"), Some(PathBuf::from("/tmp/x")));
    assert_eq!(folder_of("c"), None);
}

#[test]
fn deleted_chats_and_chats_on_other_machines_are_left_out() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::at(dir.path());
    fixture.chat(&ChatRow { id: "kept", ..ChatRow::default() });
    fixture.chat(&ChatRow { id: "gone", ..ChatRow::default() });
    fixture.delete_chat("gone");
    fixture.chat(&ChatRow { id: "ssh", host: "ssh:box", ..ChatRow::default() });
    let store = read_base(dir.path());
    let ids: Vec<&str> = store.chats.iter().map(|chat| chat.id.as_str()).collect();
    assert_eq!(ids, vec!["kept", "ssh"]);
    let live: Vec<&str> = chats(&store).map(|chat| chat.id.as_str()).collect();
    assert_eq!(live, vec!["kept"]);
}

#[test]
fn parts_take_over_the_document_only_when_current() {
    let sandbox = sandbox();
    let renamed = vec![(worktree_id(REPO, &sandbox.login), "From parts", true)];
    sandbox.fixture.worktree_parts("local-default", &renamed, 6);
    let stale = read_base(&sandbox.base);
    let name = |store: &Store| store.profiles[0].worktrees.iter().find(|worktree| worktree.path == sandbox.login).map(|worktree| worktree.name.clone());
    assert_eq!(name(&stale), Some(Some("Login page".into())), "parts written at an older revision are stale");
    sandbox.fixture.worktree_parts("local-default", &renamed, 7);
    let current = read_base(&sandbox.base);
    assert_eq!(name(&current), Some(Some("From parts".into())));
    assert_eq!(current.profiles[0].worktrees.len(), 1, "the parts are the whole map");
    assert!(current.profiles[0].worktrees[0].archived);
}

#[test]
fn worktrees_are_candidates_once_without_main_checkouts_and_archived_when_orca_archived_them() {
    let sandbox = sandbox();
    sandbox.fixture.profile("work");
    sandbox.fixture.index(&["local-default", "work"]);
    sandbox.fixture.repos("work", &[(REPO, &sandbox.main), ("ssh-repo", Path::new("/remote/app"))]);
    let remote_tree = worktree_id("ssh-repo", Path::new("/remote/app-feat"));
    sandbox.fixture.worktree_meta("work", &[(worktree_id(REPO, &sandbox.contrib), "", true), (remote_tree, "Remote", false)]);
    // The SSH repository is remote.
    let mut repos = json!([
        { "id": REPO, "path": sandbox.main, "displayName": "shop" },
        { "id": "ssh-repo", "path": "/remote/app", "connectionId": "c1" },
    ]);
    sandbox.fixture.set_document("work", "repos", &repos, 1);
    let store = read_base(&sandbox.base);
    assert_eq!(store.profiles.iter().map(|profile| profile.id.as_str()).collect::<Vec<_>>(), vec!["local-default", "work"]);
    let candidates = worktree_candidates(&store);
    let got: Vec<(PathBuf, Option<String>, bool)> = candidates.iter().map(|candidate| (candidate.path.clone(), candidate.title.clone(), candidate.archived)).collect();
    assert_eq!(
        got,
        vec![
            // Archived in one profile, not in the other: it is still used.
            (sandbox.contrib.clone(), Some("contrib".into()), false),
            (sandbox.login.clone(), Some("Login page".into()), false),
        ]
    );
    assert!(candidates.iter().all(|candidate| candidate.tool == PROVIDER && candidate.source_ref.starts_with("orca:")));
    // Archived everywhere: archived.
    repos[0]["path"] = json!(sandbox.main);
    sandbox.fixture.worktree_meta("local-default", &[(worktree_id(REPO, &sandbox.contrib), "", true)]);
    let store = read_base(&sandbox.base);
    let contrib = worktree_candidates(&store).into_iter().find(|candidate| candidate.path == sandbox.contrib).unwrap();
    assert!(contrib.archived);
    let chat = store.chats.iter().find(|chat| chat.id == "f441372b").unwrap();
    assert!(is_archived(&store, chat), "a chat in an archived worktree is archived");
    let elsewhere = store.chats.iter().find(|chat| chat.id == "16acf9ae").unwrap();
    assert!(!is_archived(&store, elsewhere), "another worktree being archived archives nothing here");
    let floating = store.chats.iter().find(|chat| chat.id == "16acf9ae").map(|chat| Chat { place: Place::None, ..chat.clone() }).unwrap();
    assert!(!is_archived(&store, &floating));
}

#[test]
fn orca_yaml_gives_setup_and_archive_scripts() {
    let scripts = store::yaml_scripts("scripts:\n  setup: |\n    pnpm install\n    cp .env.example .env\n  archive: >\n    echo\n    done\nissueCommand: x\n").unwrap();
    assert_eq!(scripts.setup.as_deref(), Some("pnpm install\ncp .env.example .env"));
    assert_eq!(scripts.archive.as_deref(), Some("echo done"));
    assert_eq!(scripts.run, None, "Orca has no run script");
    let single = store::yaml_scripts("# comment\nscripts:\n  setup: \"bun install\"\n  run: bun dev\n").unwrap();
    assert_eq!(single.setup.as_deref(), Some("bun install"));
    assert_eq!(single.run, None);
    assert_eq!(store::yaml_scripts("scripts:\n  archive: 'rm -rf build'\n").unwrap().archive.as_deref(), Some("rm -rf build"));
    let blank_line = store::yaml_scripts("scripts:\n  setup: |\n    a\n\n    b\n").unwrap();
    assert_eq!(blank_line.setup.as_deref(), Some("a\n\nb"));
    assert!(store::yaml_scripts("defaultTabs:\n  - title: x\n").is_none());
    assert!(store::yaml_scripts("scripts:\n  setup: \"\"\n").is_none());
    assert!(store::yaml_scripts("scripts:\nother: 1\n").is_none());
    assert!(store::yaml_scripts("\u{0}::::\n||\n").is_none());
}

#[test]
fn a_repositorys_orca_yaml_scripts_go_with_its_worktrees() {
    let sandbox = sandbox();
    std::fs::create_dir_all(&sandbox.main).unwrap();
    // A long file (Orca reads up to 256 KiB) is still read.
    let padding = "# a comment line to make the file long\n".repeat(100);
    std::fs::write(sandbox.main.join("orca.yaml"), format!("{padding}scripts:\n  setup: |\n    echo setup\n")).unwrap();
    let store = read_base(&sandbox.base);
    let candidates = worktree_candidates(&store);
    assert!(!candidates.is_empty());
    assert!(candidates.iter().all(|candidate| candidate.scripts.as_ref().and_then(|scripts| scripts.setup.as_deref()) == Some("echo setup")));
    // Past Orca's own limit, the file is not read.
    let huge = "# padding\n".repeat(30_000);
    std::fs::write(sandbox.main.join("orca.yaml"), format!("scripts:\n  setup: echo setup\n{huge}")).unwrap();
    let store = read_base(&sandbox.base);
    assert!(worktree_candidates(&store).iter().all(|candidate| candidate.scripts.is_none()));
}

#[test]
fn orca_times_are_epoch_milliseconds_and_zero_is_none() {
    assert_eq!(store::time_of(Some(1_795_419_158_665)).as_deref(), Some("2026-11-23T07:32:38.665+00:00"));
    assert_eq!(store::time_of(Some(1)).as_deref(), Some("1970-01-01T00:00:00.001+00:00"));
    assert_eq!(store::time_of(Some(0)), None);
    assert_eq!(store::time_of(Some(-5)), None);
    assert_eq!(store::time_of(None), None);
}

#[test]
fn the_copy_keeps_messages_reasoning_and_tool_calls_in_order() {
    let sandbox = sandbox();
    sandbox.fixture.items(
        "16acf9ae",
        "e2",
        &[
            message("u1", "user", "Read README.md and tell me what this repo is.", 1_795_419_160_874),
            turn("t1"),
            message("a1", "assistant", "I'll read the README.md file for you.", 1_795_419_165_000),
            message("r1", "reasoning", "The user wants a summary.", 1_795_419_165_100),
            tool("x1", "read", json!({ "path": "README.md" }), "1: # shop", "completed"),
            tool("x2", "bash", json!({ "command": "false" }), "exit 1", "failed"),
            subagent("s1", "a subagent's own words"),
            status("st", "Claude isn't signed in."),
            message("a2", "assistant", "It only has a title.", 1_795_419_172_193),
        ],
    );
    // An older epoch and a hidden row never come over.
    sandbox.fixture.items_at("16acf9ae", "e1", &[message("old", "user", "before /clear", 1)], false);
    sandbox.fixture.items_at("16acf9ae", "e2", &[message("hid", "user", "hidden", 1)], true);
    let items = store::items(&sandbox.fixture.chats, "16acf9ae").unwrap();
    let bodies: Vec<&Body> = items.iter().map(|item| &item.body).collect();
    assert_eq!(bodies.len(), 6, "{bodies:?}");
    assert_eq!(bodies[0], &Body::User("Read README.md and tell me what this repo is.".into()));
    assert_eq!(bodies[1], &Body::Assistant("I'll read the README.md file for you.".into()));
    assert_eq!(bodies[2], &Body::Reasoning("The user wants a summary.".into()));
    assert!(matches!(bodies[3], Body::Tool { name, output: Some(output), failed: Some(false), .. } if name == "read" && output == "1: # shop"));
    assert!(matches!(bodies[4], Body::Tool { failed: Some(true), .. }));
    assert_eq!(bodies[5], &Body::Assistant("It only has a title.".into()));
    assert_eq!(items[0].at.as_deref(), Some("2026-11-23T07:32:40.874+00:00"));

    let store = read_base(&sandbox.base);
    let chat = store.chats.iter().find(|chat| chat.id == "16acf9ae").unwrap();
    let copy = copy_chat(&store, chat, &items, "ref", &links(&store));
    assert_eq!(copy.entries.len(), 6);
    assert_eq!(copy.entries[3].kind, crate::import_bundle::BundleEntryKind::ToolCall);
    assert_eq!(copy.entries[3].tool.as_ref().unwrap().status, ToolStatus::Completed);
    assert_eq!(copy.entries[4].tool.as_ref().unwrap().status, ToolStatus::Failed);
    assert_eq!(copy.native_ref.as_ref().unwrap().key(), "opencode:ses_a");
    assert_eq!(copy.harness, "opencode");
    assert_eq!(copy.title.as_deref(), Some("What is shop?"));
    assert_eq!(copy.cwd.as_deref(), Some(&*sandbox.login.to_string_lossy()));
    assert_eq!(copy.entries_source, EntriesSource::ToolCopy);
    assert!(store::items(&sandbox.fixture.chats, "no-such-chat").unwrap().is_empty());
}

#[test]
fn a_chat_led_run_links_its_chat_and_captured_terminal_workers() {
    let sandbox = sandbox();
    sandbox.fixture.run("run_chat", Some("35dcaa0e"));
    sandbox.fixture.chat_worker("run_chat", "ctx_1", "16acf9ae");
    sandbox.fixture.terminal_worker("run_chat", "ctx_2", "claude", Some("22222222-2222-4222-8222-222222222222"));
    sandbox.fixture.terminal_worker("run_chat", "ctx_3", "opencode", None);
    sandbox.fixture.run("run_terminal", None);
    sandbox.fixture.terminal_worker("run_terminal", "ctx_4", "codex", Some("01a1f9f8-cbdc-7a20-a62a-3e214132a143"));
    let store = read_base(&sandbox.base);
    let runs = store.runs.clone().unwrap();
    let chat_run = runs.iter().find(|run| run.id == "run_chat").unwrap();
    assert_eq!(chat_run.coordinator.as_deref(), Some("35dcaa0e"));
    assert_eq!(
        chat_run.workers,
        vec![
            Worker::Chat("16acf9ae".into()),
            Worker::Session(NativeSession { harness: "claude", id: "22222222-2222-4222-8222-222222222222".into() }),
            Worker::Unknown,
        ]
    );
    let links = links(&store);
    assert_eq!(links.chats.get("16acf9ae").map(String::as_str), Some("35dcaa0e"));
    assert_eq!(links.sessions.get("claude:22222222-2222-4222-8222-222222222222").map(String::as_str), Some("35dcaa0e"));
    assert_eq!(links.coordinators, BTreeSet::from(["35dcaa0e".to_owned()]));
    assert_eq!(links.unlinked, 2, "the OpenCode terminal worker and the terminal-led run's worker");
    let items = vec![store::Item { id: "u".into(), at: None, body: Body::User("hi".into()) }];
    let by = |id: &str| store.chats.iter().find(|chat| chat.id == id).unwrap();
    let worker = copy_chat(&store, by("16acf9ae"), &items, "ref", &links);
    assert_eq!(worker.agent_role, Some(AgentRole::Worker));
    assert_eq!(worker.parent_chat_key.as_deref(), Some("35dcaa0e"));
    assert_eq!(worker.parent_native_key.as_deref(), Some("opencode:ses_b"));
    let lead = copy_chat(&store, by("35dcaa0e"), &items, "ref", &links);
    assert_eq!((lead.agent_role, lead.parent_chat_key), (Some(AgentRole::Orchestrator), None));
}

#[test]
fn a_rebound_coordinator_no_longer_leads_its_run() {
    let sandbox = sandbox();
    sandbox.fixture.run("run_1", Some("35dcaa0e"));
    sandbox.fixture.chat_worker("run_1", "ctx_1", "16acf9ae");
    sandbox.fixture.runs.execute("UPDATE runs SET consumer_generation = 2", []).unwrap();
    let store = read_base(&sandbox.base);
    assert_eq!(store.runs.unwrap()[0].coordinator, None);
}

#[test]
fn an_orchestration_database_bridge_cannot_read_is_reported_not_fatal() {
    let sandbox = sandbox();
    sandbox.fixture.runs.execute_batch("ALTER TABLE dispatch_contexts RENAME COLUMN assignee_orca_session_id TO assignee").unwrap();
    let store = read_base(&sandbox.base);
    assert_eq!(store.runs, None);
    assert!(store.notes.iter().any(|note| note.contains("multi-agent")), "{:?}", store.notes);
    assert_eq!(chats(&store).count(), 3);
}

#[test]
fn the_version_gate_fails_closed_with_the_version_it_saw() {
    let sandbox = sandbox();
    sandbox.fixture.chats.execute_batch("PRAGMA user_version = 0").unwrap();
    let error = read(&Paths::at(&sandbox.base)).unwrap_err();
    assert_eq!(error, "Orca's chat database is older than Bridge knows (version 0)");
    sandbox.fixture.chats.execute_batch("PRAGMA user_version = 2; ALTER TABLE agent_session_records RENAME COLUMN record_json TO record").unwrap();
    let error = read(&Paths::at(&sandbox.base)).unwrap_err();
    assert_eq!(error, "Orca's chat database is missing what Bridge reads (version 2)");

    let other = self::sandbox();
    other.fixture.profile("local-default").execute_batch("PRAGMA user_version = 0").unwrap();
    let error = read(&Paths::at(&other.base)).unwrap_err();
    assert_eq!(error, "Orca's profile 'local-default' is older than Bridge knows (version 0)");
    other.fixture.profile("local-default").execute_batch("PRAGMA user_version = 3").unwrap();
    other.fixture.set_document("local-default", "schemaVersion", &json!(2), 1);
    let error = read(&Paths::at(&other.base)).unwrap_err();
    assert!(error.contains("schema 2"), "{error}");
}

#[test]
fn a_newer_database_with_everything_bridge_reads_is_read_and_one_without_is_named() {
    let before = read_base(&sandbox().base);
    let sandbox = sandbox();
    sandbox.fixture.chats.execute_batch("PRAGMA user_version = 7; CREATE TABLE future_tokens(token TEXT);").unwrap();
    sandbox.fixture.profile("local-default").execute_batch("PRAGMA user_version = 9").unwrap();
    let store = read_base(&sandbox.base);
    assert_eq!(chats(&store).count(), chats(&before).count());
    assert!(chats(&store).count() > 0);

    for (chat, reads) in [(true, store::CHAT_NEWEST_READS), (false, store::PROFILE_NEWEST_READS)] {
        for (table, columns) in reads {
            for column in *columns {
                let sandbox = self::sandbox();
                let profile = sandbox.fixture.profile("local-default");
                let conn = if chat { &sandbox.fixture.chats } else { &profile };
                conn.execute_batch(&format!("PRAGMA user_version = 9; DROP TABLE IF EXISTS {table}_shadow;")).unwrap();
                // Generated columns and triggers may name the column: drop the triggers, rebuild the table without it.
                let triggers: Vec<String> =
                    conn.prepare("SELECT name FROM sqlite_master WHERE type = 'trigger'").unwrap().query_map([], |row| row.get(0)).unwrap().map(Result::unwrap).collect();
                for trigger in triggers {
                    conn.execute_batch(&format!("DROP TRIGGER \"{trigger}\"")).unwrap();
                }
                let kept: Vec<String> = conn
                    .prepare(&format!("SELECT name FROM pragma_table_info('{table}') WHERE name != ?1"))
                    .unwrap()
                    .query_map([column], |row| row.get(0))
                    .unwrap()
                    .map(Result::unwrap)
                    .collect();
                let list = kept.iter().map(|name| format!("\"{name}\"")).collect::<Vec<_>>().join(", ");
                conn.execute_batch(&format!(
                    "PRAGMA foreign_keys=OFF; CREATE TABLE {table}_shadow AS SELECT {list} FROM {table}; DROP TABLE {table}; ALTER TABLE {table}_shadow RENAME TO {table};"
                ))
                .unwrap();
                let error = read(&Paths::at(&sandbox.base)).unwrap_err();
                let label = if chat { "Orca's chat database".to_owned() } else { "Orca's profile 'local-default'".to_owned() };
                assert!(
                    error.starts_with(&format!("{label} is newer than Bridge knows (version 9) and changed what Bridge reads (")),
                    "{table}.{column}: {error}"
                );
                assert!(error.contains(&format!("{table}.{column}")), "{error}");
            }
        }
    }
}

#[test]
fn newest_reads_is_exactly_what_the_readers_read_at_the_newest_checked_versions() {
    let sandbox = sandbox();
    let version = |conn: &rusqlite::Connection| conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0)).unwrap();
    assert_eq!(version(&sandbox.fixture.chats), store::CHAT_VERSIONS.newest_checked);
    assert_eq!(version(&sandbox.fixture.profile("local-default")), store::PROFILE_VERSIONS.newest_checked);
    sandbox.fixture.items("16acf9ae", "e1", &[message("u1", "user", "hello", 1)]);
    STATEMENTS.with(|statements| statements.borrow_mut().clear());
    let (_, chats) = store::read_chats(&sandbox.fixture.chats).unwrap();
    for chat in &chats {
        store::items(&sandbox.fixture.chats, &chat.id).unwrap();
    }
    let statements = STATEMENTS.with(|statements| std::mem::take(&mut *statements.borrow_mut()));
    let allowed = crate::import_sqlite::Allowed { tool: "Orca", tables: store::CHAT_TABLES };
    crate::import_sqlite::assert_reads_match(&allowed, &sandbox.fixture.chats, store::CHAT_NEWEST_READS, &statements);
    assert!(!chats.is_empty());

    // Current split parts, so the parts query runs too.
    sandbox.fixture.worktree_parts("local-default", &[(worktree_id(REPO, &sandbox.login), "From parts", false)], 7);
    let profile = sandbox.fixture.profile("local-default");
    store::read_profile(&profile, "local-default").unwrap();
    let statements = STATEMENTS.with(|statements| std::mem::take(&mut *statements.borrow_mut()));
    let allowed = crate::import_sqlite::Allowed { tool: "Orca", tables: store::PROFILE_TABLES };
    crate::import_sqlite::assert_reads_match(&allowed, &profile, store::PROFILE_NEWEST_READS, &statements);
}

#[test]
fn only_the_old_journal_asks_to_open_orca_once() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(store::OLD_CHAT_DB), "x").unwrap();
    let error = read(&Paths::at(dir.path())).unwrap_err();
    assert!(error.contains("Open Orca once"), "{error}");
    assert!(Paths::at(dir.path()).installed());
}

#[test]
fn a_file_that_is_not_a_database_or_is_empty_says_so() {
    for content in ["not a database at all, just text", ""] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(store::CHAT_DB), content).unwrap();
        assert!(read(&Paths::at(dir.path())).is_err(), "{content:?}");
    }
}

#[test]
fn only_a_profile_index_still_finds_worktrees_and_no_chats() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::at(dir.path());
    drop(fixture);
    std::fs::remove_file(dir.path().join(store::CHAT_DB)).unwrap();
    let (store, _) = read(&Paths::at(dir.path())).unwrap();
    assert_eq!((store.chat_version, store.chats.len(), store.profiles.len()), (0, 0, 1));
    let empty = tempfile::tempdir().unwrap();
    assert!(!Paths::at(empty.path()).installed());
    assert_eq!(read(&Paths::at(empty.path())).unwrap_err(), "Orca was not found on this Mac");
}

#[test]
fn no_profiles_folder_is_nothing_to_note() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::at(dir.path());
    drop(fixture);
    std::fs::remove_dir_all(dir.path().join("profiles")).unwrap();
    let store = read_base(dir.path());
    assert!(store.profiles.is_empty());
    assert!(store.notes.is_empty(), "{:?}", store.notes);
}

#[test]
fn a_profile_on_the_old_json_file_is_skipped_and_noted() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::at(dir.path());
    drop(fixture);
    std::fs::remove_file(dir.path().join("profiles/local-default/profile-state.db")).unwrap();
    let store = read_base(dir.path());
    assert!(store.profiles.is_empty());
    assert!(store.notes.iter().any(|note| note.contains("open Orca once")), "{:?}", store.notes);
}

#[test]
fn orca_user_data_path_moves_everything() {
    let home = Path::new("/Users/me");
    assert_eq!(Paths::for_home(home, None).base, PathBuf::from("/Users/me/Library/Application Support/orca"));
    assert_eq!(Paths::for_home(home, Some(Path::new("/data/orca"))).base, PathBuf::from("/data/orca"));
    assert_eq!(Paths::at(Path::new("/d")).chats(), PathBuf::from("/d/agent-session-domain.db"));
}

#[test]
fn only_allowed_tables_and_documents_are_queried_and_no_secret_reaches_the_output() {
    let sandbox = sandbox();
    sandbox.fixture.run("run_chat", Some("35dcaa0e"));
    sandbox.fixture.terminal_worker("run_chat", "ctx_2", "claude", Some("22222222-2222-4222-8222-222222222222"));
    sandbox.fixture.items("16acf9ae", "e1", &[message("u1", "user", "hello", 1)]);
    STATEMENTS.with(|statements| statements.borrow_mut().clear());
    let (store, path) = read(&Paths::at(&sandbox.base)).unwrap();
    let items = store::items(&sandbox.fixture.chats, "16acf9ae").unwrap();
    let statements = STATEMENTS.with(|statements| statements.borrow().clone());
    let allowed: Vec<&str> = store::CHAT_TABLES.iter().chain(store::PROFILE_TABLES).chain(store::ORCHESTRATION_TABLES).copied().collect();
    for statement in &statements {
        if statement == "PRAGMA user_version" {
            continue;
        }
        let named = allowed.iter().any(|table| statement.contains(&format!("FROM {table}")) || statement.contains(&format!("pragma_table_info('{table}')")));
        assert!(named, "{statement}");
        assert!(!statement.contains("SELECT \"record_json\"") && !statement.contains("SELECT \"resume_session\""), "whole JSON columns stay in SQLite: {statement}");
    }
    let documents: Vec<&String> = statements.iter().filter(|statement| statement.contains("FROM profile_state_documents")).collect();
    assert!(documents.iter().all(|statement| statement.contains("domain IN")), "{documents:?}");
    let links = links(&store);
    let output = format!(
        "{store:?}{:?}{:?}{:?}",
        worktree_candidates(&store),
        chats(&store).map(|chat| copy_chat(&store, chat, &items, &source_ref(&path, &chat.id), &links)).collect::<Vec<_>>(),
        counts(&store),
    );
    assert!(!output.contains(CANARY), "a secret reached the output");
    // The planted secret files are still unreadable: nothing needed them.
    for name in store::DENIED_FILES {
        let at = if matches!(*name, "account-session.json.enc" | "browser-passwords") { sandbox.base.join("profiles/local-default").join(name) } else { sandbox.base.join(name) };
        let file = if name.contains('.') { at } else { at.join("secret.json") };
        assert!(std::fs::read(&file).is_err(), "{name} is planted unreadable");
    }
}

#[test]
fn a_closed_database_is_read_without_creating_a_file_and_a_live_wal_is_read_through() {
    let sandbox = sandbox();
    let Sandbox { base, fixture, _dir, .. } = sandbox;
    drop(fixture);
    let listing = || {
        let mut names: Vec<String> = std::fs::read_dir(&base).unwrap().flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    };
    let before = listing();
    assert!(!before.iter().any(|name| name.ends_with("-wal")), "{before:?}");
    let store = read_base(&base);
    assert_eq!(chats(&store).count(), 3);
    assert_eq!(listing(), before, "reading created nothing");

    let live = Connection::open(base.join(store::CHAT_DB)).unwrap();
    live.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;").unwrap();
    live.execute("DELETE FROM agent_session_records WHERE session_id='f441372b'", []).unwrap();
    assert!(std::fs::metadata(base.join(format!("{}-wal", store::CHAT_DB))).unwrap().len() > 0);
    assert_eq!(chats(&read_base(&base)).count(), 2, "the live WAL's change is seen");
}

#[test]
fn copies_are_planned_only_for_chats_whose_session_was_not_found() {
    let sandbox = sandbox();
    let store = read_base(&sandbox.base);
    let plan = plan_chats(&store, &HashSet::from([opencode("ses_a")]));
    assert_eq!(plan.native, BTreeSet::from([opencode("ses_a")]));
    assert_eq!(plan.copies, vec!["35dcaa0e".to_owned(), "f441372b".to_owned()]);
}

#[test]
fn account_folders_are_the_ones_each_agent_reads() {
    let chat = |agent: &str, variable: &str| Chat {
        id: "c".into(),
        agent: agent.into(),
        place: Place::None,
        launch_directory: None,
        local: true,
        title: None,
        renamed: false,
        created_at: None,
        updated_at: None,
        model: None,
        native: None,
        account_home: Some(store::AccountHome { variable: variable.into(), path: PathBuf::from("/a") }),
    };
    assert_eq!(account_root(&chat("claude", "CLAUDE_CONFIG_DIR")), Some(("claude", PathBuf::from("/a"))));
    assert_eq!(account_root(&chat("claude", "HOME")), Some(("claude", PathBuf::from("/a/.claude"))));
    assert_eq!(account_root(&chat("codex", "CODEX_HOME")), Some(("codex", PathBuf::from("/a"))));
    assert_eq!(account_root(&chat("codex", "HOME")), Some(("codex", PathBuf::from("/a/.codex"))));
    assert_eq!(account_root(&chat("codex", "CLAUDE_CONFIG_DIR")), None);
    assert_eq!(account_root(&chat("opencode", "HOME")), None);
    assert_eq!(account_root(&Chat { account_home: None, ..chat("claude", "HOME") }), None);
}

#[test]
fn source_refs_round_trip() {
    let path = Path::new("/a/orca/agent-session-domain.db");
    let reference = source_ref(path, "16acf9ae");
    assert_eq!(parse_source_ref(&reference), Some((path.to_path_buf(), "16acf9ae".to_owned())));
    assert_eq!(parse_source_ref("no-hash"), None);
    assert_eq!(parse_source_ref("#x"), None);
    assert_eq!(parse_source_ref("/a#"), None);
}

#[test]
fn only_orcas_own_safe_candidates_are_normalized() {
    let sandbox = sandbox();
    sandbox.fixture.items("16acf9ae", "e1", &[message("u1", "user", "hello", 1), message("a1", "assistant", "hi", 2)]);
    let (store, path) = read(&Paths::at(&sandbox.base)).unwrap();
    let artifacts: Vec<DiscoveredArtifact> = chats(&store).map(|chat| copy_artifact(&path, &store, chat)).collect();
    let discovery = job_discovery(&[sandbox.base.clone()], artifacts);
    let selection = DiscoverySelection { artifact_ids: discovery.artifacts.iter().map(|artifact| artifact.artifact_id.clone()).collect() };
    let candidates = OrcaImporter.preview(&discovery, &selection).unwrap();
    let conversations: Vec<&ImportCandidate> = candidates.iter().filter(|candidate| candidate.kind == CandidateKind::Conversation).collect();
    assert_eq!(conversations.len(), 1, "{candidates:?}");
    assert_eq!(candidates.iter().filter(|candidate| candidate.kind == CandidateKind::Unsupported).count(), 2, "the two chats with no copy");
    let normalized = OrcaImporter.normalize(conversations[0].clone()).unwrap();
    assert_eq!(normalized.candidate.source.provider, PROVIDER);
    let unsupported = candidates.iter().find(|candidate| candidate.kind == CandidateKind::Unsupported).unwrap().clone();
    assert!(OrcaImporter.normalize(unsupported).is_err());
    let mut other = conversations[0].clone();
    other.source.provider = "t3code".into();
    assert!(OrcaImporter.normalize(other.clone()).is_err());
    assert!(!OrcaImporter.validate(&other, &SchemaGate { format: STORE_GATE.into(), allowed_versions: vec![STORE_GATE.into()] }).accepted);
    // A reference outside the approved folders, or to another file, is refused.
    let outside = tempfile::tempdir().unwrap();
    let mut stray = discovery.clone();
    stray.approved_roots = vec![outside.path().to_string_lossy().into_owned()];
    let refused = OrcaImporter.preview(&stray, &selection).unwrap();
    assert!(refused.iter().all(|candidate| candidate.kind == CandidateKind::Unsupported));
    let mut wrong = discovery.clone();
    wrong.provider = "t3code".into();
    assert!(OrcaImporter.preview(&wrong, &selection).is_err());
    assert!(OrcaImporter.discover(&DiscoveryRequest { provider: "t3code".into(), approved_roots: Vec::new(), selected_export: None, source_version: None, schema_version: None, format_versions: BTreeMap::new() }).is_err());
}

#[test]
fn discover_lists_every_live_chat() {
    let sandbox = sandbox();
    let discovery = OrcaImporter
        .discover(&DiscoveryRequest { provider: PROVIDER.into(), approved_roots: vec![sandbox.base.to_string_lossy().into_owned()], selected_export: None, source_version: None, schema_version: None, format_versions: BTreeMap::new() })
        .unwrap();
    assert_eq!(discovery.artifacts.len(), 3);
    assert!(discovery.artifacts.iter().all(|artifact| artifact.canonical_source_ref.ends_with(&format!("{}#{}", store::CHAT_DB, artifact.source_label.trim_start_matches("Orca/")))));
    assert_eq!(OrcaImporter.fixture_manifest().provider, PROVIDER);
}

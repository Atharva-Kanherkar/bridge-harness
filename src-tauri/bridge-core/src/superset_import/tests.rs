use super::fixture::{agent, item, reasoning, session_event, tool, turn, user, ChatRow, Fixture, WorkspaceRow, CANARY, CHAT_HEADS, HOST_HEADS, ORG};
use super::store::{self, Body, Kind, NativeSession};
use super::*;
use crate::import_sqlite::STATEMENTS;

fn opencode(id: &str) -> NativeSession {
    NativeSession { harness: "opencode", id: id.into() }
}

/// An app home like the sandbox's: a project with two worktree workspaces
/// (one archived), a local and a session workspace, an OpenCode chat, a
/// Claude Code chat that never started, terminal sessions with and without
/// a session, a tag, and an agent configuration with a key.
struct Sandbox {
    _dir: tempfile::TempDir,
    home: PathBuf,
    repo: PathBuf,
    login: PathBuf,
    fixture: Fixture,
}

fn sandbox() -> Sandbox {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join(".paradev");
    let repo = dir.path().join("code/shop");
    let login = home.join("worktrees/shop/feat-login");
    let fixture = Fixture::newest(&home);
    fixture.project("p1", &repo, "shop");
    let workspace = |id, path: &Path, kind, archived, name| {
        fixture.workspace(&WorkspaceRow { id, project: Some("p1"), path, branch: "b", name, kind, archived });
    };
    workspace("w-login", &login, "worktree", false, "Login page");
    workspace("w-contrib", &home.join("worktrees/shop/contrib"), "worktree", false, "Contributing guide");
    workspace("w-old", &home.join("worktrees/shop/old-spike"), "worktree", true, "Old spike");
    workspace("w-local", &repo, "local", false, "shop");
    fixture.workspace(&WorkspaceRow { id: "w-scratch", project: None, path: &home.join("sessions/scratch"), branch: "main", name: "Scratch", kind: "session", archived: false });
    fixture.chat(&ChatRow { id: "c-open", workspace: "w-login", native: Some("ses_a"), ..ChatRow::default() });
    fixture.chat(&ChatRow { id: "c-claude", workspace: "w-login", harness: "claude-acp", ..ChatRow::default() });
    fixture.chat(&ChatRow { id: "c-pi", workspace: "w-login", harness: "pi-acp", native: Some("pi-1"), ..ChatRow::default() });
    fixture.binding("t-ran", "w-contrib", "opencode", Some("ses_t"), None);
    fixture.binding("t-failed", "w-contrib", "opencode", None, None);
    fixture.binding("t-amp", "w-contrib", "amp", Some("amp-1"), None);
    fixture.tag("w-contrib", "docs");
    fixture.config("cfg-key", Some("claude"), "Claude (work key)", &json!({ "ANTHROPIC_API_KEY": CANARY }));
    fixture.config("cfg-custom", Some("custom"), "My agent", &json!({}));
    Sandbox { _dir: dir, home, repo, login, fixture }
}

fn read_home(home: &Path) -> Store {
    read(&Paths::at(home)).unwrap()
}

#[test]
fn the_newest_schemas_read_projects_workspaces_chats_bindings_and_configs() {
    let sandbox = sandbox();
    let store = read_home(&sandbox.home);
    assert_eq!(store.orgs.len(), 1);
    let org = &store.orgs[0];
    assert_eq!(org.id, ORG);
    assert_eq!((org.host.version, org.chat_version), (store::HOST_NEWEST, Some(store::CHAT_NEWEST)));
    assert_eq!(org.host.projects["p1"].root, sandbox.repo);
    assert_eq!(org.host.workspaces["w-login"].name.as_deref(), Some("Login page"));
    assert_eq!(org.host.workspaces["w-local"].kind, Kind::Local);
    assert_eq!(org.host.workspaces["w-scratch"].kind, Kind::Session);
    assert!(org.host.workspaces["w-old"].archived);
    let live: Vec<&str> = chats(&store).map(|(_, chat)| chat.id.as_str()).collect();
    assert_eq!(live, vec!["c-claude", "c-open"], "Pi is an agent Bridge does not run");
    let terminal: Vec<&str> = bindings(&store).map(|(_, binding)| binding.terminal.as_str()).collect();
    assert_eq!(terminal, vec!["t-ran"]);
    assert_eq!(org.chats.iter().find(|chat| chat.id == "c-open").unwrap().native, Some(opencode("ses_a")));
    assert_eq!(folder(org, "w-login"), Some(sandbox.login.clone()));
    assert_eq!(org.host.tags, 1);
    assert_eq!(counts(&store), Counts { chats: 3, projects: 1, workspaces: 2 });
    let candidates = worktree_candidates(&store);
    let got: Vec<(String, Option<String>, bool)> = candidates.iter().map(|candidate| (candidate.name.clone(), candidate.title.clone(), candidate.archived)).collect();
    assert_eq!(
        got,
        vec![
            ("contrib".into(), Some("Contributing guide".into()), false),
            ("feat-login".into(), Some("Login page".into()), false),
            ("old-spike".into(), Some("Old spike".into()), true),
        ],
        "local and session workspaces are not candidates"
    );
}

#[test]
fn every_rebuilt_head_reads_with_the_columns_it_has() {
    for &host in HOST_HEADS {
        for &chat in CHAT_HEADS {
            let dir = tempfile::tempdir().unwrap();
            let fixture = Fixture::at(dir.path(), host, Some(chat));
            fixture.project("p1", Path::new("/code/shop"), "shop");
            fixture.workspace(&WorkspaceRow { id: "w1", project: Some("p1"), path: Path::new("/w/shop/a"), branch: "a", name: "A", kind: "worktree", archived: true });
            fixture.chat(&ChatRow { id: "c1", workspace: "w1", native: Some("ses_1"), ..ChatRow::default() });
            fixture.events("c1", "e1", &[user("u1", "hi"), agent("a1", "hello")]);
            let has_bindings = host >= 6;
            if has_bindings {
                fixture.binding("t1", "w1", "claude", Some("11111111-1111-4111-8111-111111111111"), None);
            }
            let store = read_home(dir.path());
            let org = &store.orgs[0];
            assert_eq!(org.host.projects.len(), 1, "host {host}");
            let workspace = &org.host.workspaces["w1"];
            assert_eq!(workspace.path, PathBuf::from("/w/shop/a"));
            // Archiving arrived in 0020 (archived_at); before it nothing was archived.
            assert_eq!(workspace.archived, host >= 20, "host {host}");
            assert_eq!(org.chats[0].native, Some(opencode("ses_1")), "chat {chat}");
            assert_eq!(bindings(&store).count(), usize::from(has_bindings), "host {host}");
            let items = store::items(fixture.chat_conn(), "c1", "e1").unwrap();
            assert_eq!(items.len(), 2, "chat {chat}");
        }
    }
}

#[test]
fn chat_harnesses_and_terminal_agents_map_to_what_bridge_runs() {
    for (harness, want) in [("claude-acp", Some("claude")), ("claude-code", Some("claude")), ("codex-acp", Some("codex")), ("codex", Some("codex")), ("opencode-acp", Some("opencode"))] {
        assert_eq!(store::harness_of_chat(harness), want, "{harness}");
    }
    for harness in ["pi-acp", "pdev-acp", "gemini-acp", "", "claude"] {
        assert_eq!(store::harness_of_chat(harness), None, "{harness}");
    }
    for (agent, want) in [("claude", Some("claude")), ("codex", Some("codex")), ("opencode", Some("opencode")), ("amp", None), ("cursor-agent", None), ("claude-acp", None)] {
        assert_eq!(store::harness_of_agent(agent), want, "{agent}");
    }
}

#[test]
fn the_copy_keeps_each_items_latest_body_where_it_first_appeared() {
    let sandbox = sandbox();
    let mut hidden = user("u2", "visible");
    hidden["item"]["content"] = json!([{ "type": "text", "text": "secret prompt", "hidden": true }, { "type": "text", "text": "<superset-context>cwd: /x</superset-context>\nAnd also this." }]);
    let mut discarded = user("u3", "never sent");
    discarded["item"]["discarded"] = json!(true);
    let mut subagent = agent("s1", "a subagent's words");
    subagent["item"]["parentItemId"] = json!("x1");
    sandbox.fixture.events(
        "c-open",
        "e1",
        &[
            session_event("starting"),
            user("u1", "Read README.md"),
            turn("running"),
            agent("a1", ""),
            reasoning("r1", "The user wants a summary."),
            tool("x1", "read", json!({ "path": "README.md" }), "1: # shop", "running"),
            agent("a1", "It is an empty scaffold."),
            tool("x1", "read", json!({ "path": "README.md" }), "1: # shop", "completed"),
            user("u1", "Read README.md"),
            subagent,
            item(json!({ "id": "p1", "kind": "plan", "entries": [] })),
            item(json!({ "id": "n1", "kind": "notice", "text": "Claude isn't signed in" })),
            hidden,
            discarded,
            turn("completed"),
        ],
    );
    sandbox.fixture.events("c-open", "e0", &[user("old", "before the epoch")]);
    let items = store::items(sandbox.fixture.chat_conn(), "c-open", "e1").unwrap();
    let bodies: Vec<&Body> = items.iter().map(|item| &item.body).collect();
    assert_eq!(bodies.len(), 5, "{bodies:?}");
    assert_eq!(bodies[0], &Body::User("Read README.md".into()), "first seen first, once");
    assert_eq!(bodies[1], &Body::Assistant("It is an empty scaffold.".into()), "the latest body, at its first place");
    assert_eq!(bodies[2], &Body::Reasoning("The user wants a summary.".into()));
    assert!(matches!(bodies[3], Body::Tool { name, output: Some(output), failed: Some(false), .. } if name == "read" && output == "1: # shop"));
    assert_eq!(bodies[4], &Body::User("And also this.".into()), "hidden parts and Superset's context are not the person's words");
    assert_eq!(items[0].at.as_deref(), Some("2026-11-23T09:45:48.315+00:00"));

    let store = read_home(&sandbox.home);
    let org = &store.orgs[0];
    let chat = org.chats.iter().find(|chat| chat.id == "c-open").unwrap();
    let copy = copy_chat(org, chat, &items, "ref");
    assert_eq!(copy.entries.len(), 5);
    assert_eq!(copy.entries[3].tool.as_ref().unwrap().status, ToolStatus::Completed);
    assert_eq!(copy.native_ref.as_ref().unwrap().key(), "opencode:ses_a");
    assert_eq!((copy.harness.as_str(), copy.title.as_deref(), copy.archived), ("opencode", None, false));
    assert_eq!(copy.cwd.as_deref(), Some(&*sandbox.login.to_string_lossy()));
    assert_eq!(copy.entries_source, EntriesSource::ToolCopy);
    assert!(store::items(sandbox.fixture.chat_conn(), "c-open", "no-such-epoch").unwrap().is_empty());
}

#[test]
fn tool_calls_read_their_terminal_output_raw_output_and_failure() {
    let terminal = json!({ "id": "x", "kind": "tool_call", "toolName": "bash", "status": "failed", "content": [{ "type": "terminal", "command": "false", "output": "exit 1", "exitCode": 1 }] });
    assert!(matches!(store::item_of(&terminal).unwrap().body, Body::Tool { output: Some(ref output), failed: Some(true), .. } if output == "exit 1"));
    let raw = json!({ "id": "y", "kind": "tool_call", "title": "Search", "status": "pending", "rawOutput": { "hits": 3 } });
    assert!(matches!(store::item_of(&raw).unwrap().body, Body::Tool { ref name, output: Some(ref output), failed: None, .. } if name == "Search" && output == "{\"hits\":3}"));
    let diff = json!({ "id": "z", "kind": "tool_call", "toolKind": "edit", "status": "completed", "content": [{ "type": "diff", "path": "a.ts", "oldText": "a", "newText": "b" }] });
    assert!(matches!(store::item_of(&diff).unwrap().body, Body::Tool { ref name, output: Some(ref output), .. } if name == "edit" && output == "Edited a.ts"));
    assert!(store::item_of(&json!({ "id": "n", "kind": "tool_call" })).is_none(), "no name");
    assert!(store::item_of(&json!({ "kind": "agent_message", "text": "no id" })).is_none());
    assert!(store::item_of(&json!({ "id": "e", "kind": "agent_message", "text": "  " })).is_none());
    assert_eq!(store::item_of(&json!({ "id": "r", "kind": "reasoning", "summary": "short" })).unwrap().body, Body::Reasoning("short".into()));
    assert!(store::item_of(&json!({ "id": "u", "kind": "user_message", "content": [{ "type": "text", "text": "<superset-context>only</superset-context>" }] })).is_none());
    assert_eq!(
        store::item_of(&json!({ "id": "u", "kind": "user_message", "content": [{ "type": "text", "text": "a <superset-context>x" }] })).unwrap().body,
        Body::User("a".into()),
        "an unclosed context block ends the text"
    );
}

#[test]
fn the_version_gate_fails_closed_with_the_version_it_saw() {
    let sandbox = sandbox();
    let bump = |conn: &Connection, when: i64| conn.execute("INSERT INTO __drizzle_migrations(hash, created_at) VALUES('h', ?1)", [when]).unwrap();
    bump(&sandbox.fixture.host, store::HOST_NEWEST + 1);
    let error = read(&Paths::at(&sandbox.home)).unwrap_err();
    assert!(error.contains("newer") && error.contains(&(store::HOST_NEWEST + 1).to_string()), "{error}");
    sandbox.fixture.host.execute("DELETE FROM __drizzle_migrations", []).unwrap();
    let error = read(&Paths::at(&sandbox.home)).unwrap_err();
    assert!(error.contains("no version"), "{error}");
    bump(&sandbox.fixture.host, store::HOST_OLDEST - 1);
    let error = read(&Paths::at(&sandbox.home)).unwrap_err();
    assert!(error.contains("older"), "{error}");

    let other = self::sandbox();
    bump(other.fixture.chat_conn(), store::CHAT_NEWEST + 1);
    let error = read(&Paths::at(&other.home)).unwrap_err();
    assert!(error.contains("chats") && error.contains("newer"), "{error}");

    let renamed = self::sandbox();
    renamed.fixture.host.execute_batch("ALTER TABLE workspaces RENAME COLUMN worktree_path TO path").unwrap();
    assert!(read(&Paths::at(&renamed.home)).unwrap_err().contains("missing what Bridge reads"));
    let renamed = self::sandbox();
    renamed.fixture.chat_conn().execute_batch("ALTER TABLE chat_sessions_local RENAME COLUMN scope_id TO scope").unwrap();
    assert!(read(&Paths::at(&renamed.home)).unwrap_err().contains("missing what Bridge reads"));
}

#[test]
fn a_file_that_is_not_a_database_or_is_empty_says_so() {
    for content in ["not a database at all, just text", ""] {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("host").join(ORG);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(store::HOST_DB), content).unwrap();
        assert!(read(&Paths::at(dir.path())).is_err(), "{content:?}");
    }
}

#[test]
fn homes_follow_superset_and_count_a_linked_home_once() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let env_home = Path::new("/data/superset");
    assert_eq!(Paths::for_home(home, Some(env_home), Some(Path::new("/other"))).homes, vec![env_home.to_path_buf()], "SUPERSET_ wins");
    assert_eq!(Paths::for_home(home, None, Some(Path::new("/other"))).homes, vec![PathBuf::from("/other")]);
    assert!(Paths::for_home(home, None, None).homes.is_empty(), "nothing there");
    std::fs::create_dir_all(home.join(".superset/host")).unwrap();
    assert_eq!(Paths::for_home(home, None, None).homes, vec![home.join(".superset")], "the older home alone is read");
    std::fs::remove_dir_all(home.join(".superset")).unwrap();
    let fixture = Fixture::newest(&home.join(".paradev"));
    drop(fixture);
    std::os::unix::fs::symlink(".paradev", home.join(".superset")).unwrap();
    let paths = Paths::for_home(home, None, None);
    assert_eq!(paths.homes, vec![home.join(".paradev")], "a symlink to it is the same home");
    assert_eq!(paths.organizations().len(), 1);
    // An organization folder without a database, and the standalone one.
    std::fs::create_dir_all(home.join(".paradev/host/standalone")).unwrap();
    std::fs::create_dir_all(home.join(".paradev/host/empty-org")).unwrap();
    assert_eq!(paths.organizations().iter().map(|org| org.id.as_str()).collect::<Vec<_>>(), vec![ORG]);
    assert!(paths.installed());
    assert!(!Paths::at(&home.join("nowhere")).installed());
    assert_eq!(read(&Paths::at(&home.join("nowhere"))).unwrap_err(), "Superset was not found on this Mac");
}

#[test]
fn two_homes_holding_one_organization_read_it_once() {
    let dir = tempfile::tempdir().unwrap();
    let first = Fixture::newest(&dir.path().join("a"));
    let second = Fixture::newest(&dir.path().join("b"));
    first.project("p-a", Path::new("/a"), "a");
    second.project("p-b", Path::new("/b"), "b");
    let store = read(&Paths { homes: vec![dir.path().join("a"), dir.path().join("b")] }).unwrap();
    assert_eq!(store.orgs.len(), 1);
    assert!(store.orgs[0].host.projects.contains_key("p-a"), "the first home wins");
}

#[test]
fn a_project_config_gives_setup_run_and_archive_scripts() {
    let scripts = store::json_scripts(r#"{"setup":["bun install","cp .env.example .env"],"teardown":["echo bye"],"run":["bun dev"]}"#).unwrap();
    assert_eq!((scripts.setup.as_deref(), scripts.run.as_deref(), scripts.archive.as_deref()), (Some("bun install\ncp .env.example .env"), Some("bun dev"), Some("echo bye")));
    assert!(store::json_scripts(r#"{"setup":[],"cwd":"x"}"#).is_none());
    assert!(store::json_scripts("not json").is_none());
    assert_eq!(store::json_scripts(r#"{"setup":"one line"}"#).unwrap().setup.as_deref(), Some("one line"));
    let toml = store::toml_scripts("[install]\nrun = \"bun install\"\n[setup]\nrun = [\"cp .env.example .env\"]\n[archive]\nrun = \"echo bye\"\n[terminals.web]\nrun = \"bun web\"\n[terminals.dev]\nrun = \"bun dev\"\n").unwrap();
    assert_eq!((toml.setup.as_deref(), toml.run.as_deref(), toml.archive.as_deref()), (Some("bun install\ncp .env.example .env"), Some("bun dev"), Some("echo bye")));
    let default = store::toml_scripts("[terminals.web]\nrun = \"bun web\"\ndefault = true\n[terminals.dev]\nrun = \"bun dev\"\n").unwrap();
    assert_eq!(default.run.as_deref(), Some("bun web"), "the default terminal wins");
    assert!(store::toml_scripts("[ports.web]\ndefault = 3000\n").is_none());
    assert!(store::toml_scripts("= garbage").is_none());

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    assert!(store::project_scripts(repo).is_none());
    std::fs::create_dir_all(repo.join(".superset")).unwrap();
    std::fs::write(repo.join(".superset/config.json"), r#"{"setup":["old"]}"#).unwrap();
    assert_eq!(store::project_scripts(repo).unwrap().setup.as_deref(), Some("old"), "the older folder is read");
    std::fs::create_dir_all(repo.join(".paradev")).unwrap();
    std::fs::write(repo.join(".paradev/config.json"), r#"{"setup":["new"]}"#).unwrap();
    assert_eq!(store::project_scripts(repo).unwrap().setup.as_deref(), Some("new"), ".paradev wins");
    std::fs::write(repo.join(".paradev/config.toml"), "[setup]\nrun = \"toml\"\n").unwrap();
    assert_eq!(store::project_scripts(repo).unwrap().setup.as_deref(), Some("toml"), "TOML over JSON");
}

#[test]
fn a_projects_config_goes_with_its_worktrees() {
    let sandbox = sandbox();
    std::fs::create_dir_all(sandbox.repo.join(".paradev")).unwrap();
    std::fs::write(sandbox.repo.join(".paradev/config.json"), r#"{"setup":["bun install"],"teardown":["echo bye"]}"#).unwrap();
    let store = read_home(&sandbox.home);
    assert!(worktree_candidates(&store).iter().all(|candidate| candidate.scripts.as_ref().and_then(|scripts| scripts.archive.as_deref()) == Some("echo bye")));
}

#[test]
fn chats_in_a_deleted_project_and_archived_workspaces() {
    let sandbox = sandbox();
    sandbox.fixture.chat(&ChatRow { id: "c-old", workspace: "w-old", native: Some("ses_old"), ..ChatRow::default() });
    let store = read_home(&sandbox.home);
    let org = &store.orgs[0];
    assert!(is_archived(org, "w-old"));
    assert!(!is_archived(org, "w-login"));
    assert!(chats(&store).any(|(_, chat)| chat.id == "c-old"), "an archived workspace's chat still comes over");
    sandbox.fixture.delete_project("p1");
    let store = read_home(&sandbox.home);
    assert_eq!(chats(&store).count(), 0);
    assert_eq!(bindings(&store).count(), 0);
    assert!(worktree_candidates(&store).is_empty());
    assert_eq!(counts(&store), Counts::default());
}

#[test]
fn a_terminal_session_a_chat_already_names_is_one_chat() {
    let sandbox = sandbox();
    sandbox.fixture.binding("t-same", "w-login", "opencode", Some("ses_a"), None);
    let store = read_home(&sandbox.home);
    let terminal: Vec<&str> = bindings(&store).map(|(_, binding)| binding.terminal.as_str()).collect();
    assert_eq!(terminal, vec!["t-ran"]);
    assert_eq!(counts(&store).chats, 3);
}

#[test]
fn the_plan_reads_found_sessions_natively_and_copies_the_rest() {
    let sandbox = sandbox();
    let store = read_home(&sandbox.home);
    assert_eq!(wanted(&store), BTreeSet::from([opencode("ses_a"), opencode("ses_t")]));
    let plan = plan_chats(&store, &HashSet::from([opencode("ses_a")]));
    assert_eq!(plan.native, BTreeSet::from([opencode("ses_a")]));
    assert_eq!(plan.copies, vec![(ORG.to_owned(), "c-claude".to_owned())]);
    assert_eq!(plan.gone, 1, "the terminal session's transcript is gone");
    let plan = plan_chats(&store, &HashSet::from([opencode("ses_t")]));
    assert_eq!(plan.gone, 0);
    assert_eq!(plan.copies.len(), 2);
}

#[test]
fn extra_roots_are_the_claude_folder_a_transcript_is_in_and_superset_defaults() {
    let sandbox = sandbox();
    sandbox.fixture.binding("t-claude", "w-login", "claude", Some("11111111-1111-4111-8111-111111111111"), Some(Path::new("/acct/claude/projects/-w/11111111-1111-4111-8111-111111111111.jsonl")));
    sandbox.fixture.binding("t-odd", "w-login", "claude", Some("22222222-2222-4222-8222-222222222222"), Some(Path::new("/x/y.jsonl")));
    sandbox.fixture.host.execute("INSERT INTO host_settings(id, default_claude_config_dir, default_codex_home) VALUES(1, '/acct/default-claude', '/acct/codex')", []).unwrap();
    let store = read_home(&sandbox.home);
    assert_eq!(
        extra_roots(&store),
        vec![("claude", PathBuf::from("/acct/claude")), ("claude", PathBuf::from("/acct/default-claude")), ("codex", PathBuf::from("/acct/codex"))]
    );
}

#[test]
fn agent_configurations_you_made_come_over_with_key_names_only() {
    let sandbox = sandbox();
    let store = read_home(&sandbox.home);
    let configs = &store.orgs[0].host.configs;
    assert_eq!(configs.len(), 3, "the seeded one is read too");
    let own: Vec<&str> = own_configs(&store).map(|(_, config)| config.label.as_str()).collect();
    assert_eq!(own, vec!["Claude (work key)", "My agent"], "Superset's seeded built-in is not yours");
    let keyed = configs.iter().find(|config| config.id == "cfg-key").unwrap();
    assert_eq!(keyed.env_names, vec!["ANTHROPIC_API_KEY".to_owned()]);
    assert_eq!(keyed.args, vec!["--verbose".to_owned()]);
    let setup = config_setup(keyed, "ref");
    assert_eq!(setup.kind, CandidateKind::ProviderInstance);
    assert_eq!(setup.config.as_ref().unwrap()["envNames"], json!(["ANTHROPIC_API_KEY"]));
    assert!(!format!("{setup:?}").contains(CANARY));
}

#[test]
fn garbage_in_an_environment_column_reads_as_no_names() {
    let sandbox = sandbox();
    sandbox.fixture.config("cfg-bad", Some("custom"), "Bad", &json!("not an object"));
    sandbox.fixture.host.execute("UPDATE host_agent_configs SET env_json = 'not json' WHERE id = 'cfg-custom'", []).unwrap();
    let store = read_home(&sandbox.home);
    let names = |id: &str| store.orgs[0].host.configs.iter().find(|config| config.id == id).unwrap().env_names.clone();
    assert!(names("cfg-bad").is_empty());
    assert!(names("cfg-custom").is_empty());
}

#[test]
fn only_allowed_tables_are_queried_and_no_secret_reaches_the_output() {
    let sandbox = sandbox();
    sandbox.fixture.events("c-claude", "e1", &[user("u1", "Say hi")]);
    STATEMENTS.with(|statements| statements.borrow_mut().clear());
    let store = read_home(&sandbox.home);
    let org = &store.orgs[0];
    let items = store::items(sandbox.fixture.chat_conn(), "c-claude", "e1").unwrap();
    let statements = STATEMENTS.with(|statements| statements.borrow().clone());
    let allowed: Vec<&str> = store::HOST_TABLES.iter().chain(store::CHAT_TABLES).copied().collect();
    for statement in &statements {
        let named = allowed.iter().any(|table| statement.contains(&format!("FROM {table}")) || statement.contains(&format!("pragma_table_info('{table}')")));
        assert!(named, "{statement}");
        if statement.contains("env_json") {
            assert!(statement.contains("json_each") && !statement.contains("SELECT \"env_json\""), "env values never leave SQLite: {statement}");
        }
        assert!(!statement.contains("mobile_") && !statement.contains("pull_requests\"") && !statement.contains("chat_sync_state"), "{statement}");
    }
    let chat = org.chats.iter().find(|chat| chat.id == "c-claude").unwrap();
    let output = format!(
        "{store:?}{:?}{:?}{:?}{:?}",
        worktree_candidates(&store),
        copy_chat(org, chat, &items, "ref"),
        own_configs(&store).map(|(_, config)| config_setup(config, "ref")).collect::<Vec<_>>(),
        counts(&store),
    );
    assert!(!output.contains(CANARY), "a secret reached the output");
    for file in ["auth-token.enc", "config.json", "terminal-host.token", "cloud-workspace-tickets.json", "plugin-mcp-sessions.json", "local.db", "state/default-claude-config-dir", "agent/mcp.json"] {
        assert!(std::fs::read(sandbox.home.join(file)).is_err(), "{file} is planted unreadable");
    }
    assert!(std::fs::read(sandbox.fixture.folder.join("manifest.json")).is_err());
}

#[test]
fn a_closed_database_is_read_without_creating_a_file_and_a_wal_left_behind_is_read_through() {
    let sandbox = sandbox();
    let Sandbox { home, fixture, _dir, .. } = sandbox;
    let folder = fixture.folder.clone();
    // Superset's host quits without checkpointing: copy the live files as it leaves them.
    fixture.host.execute_batch("PRAGMA wal_autocheckpoint=0;").unwrap();
    fixture.host.execute("UPDATE workspaces SET name = 'Renamed' WHERE id = 'w-login'", []).unwrap();
    let left = tempfile::tempdir().unwrap();
    let left_folder = left.path().join("host").join(ORG);
    std::fs::create_dir_all(&left_folder).unwrap();
    for suffix in ["", "-wal", "-shm"] {
        for name in [store::HOST_DB, store::CHAT_DB] {
            let from = folder.join(format!("{name}{suffix}"));
            if from.is_file() {
                std::fs::copy(&from, left_folder.join(format!("{name}{suffix}"))).unwrap();
            }
        }
    }
    assert!(std::fs::metadata(left_folder.join(format!("{}-wal", store::HOST_DB))).unwrap().len() > 0);
    let store = read(&Paths::at(left.path())).unwrap();
    assert_eq!(store.orgs[0].host.workspaces["w-login"].name.as_deref(), Some("Renamed"), "the WAL's change is read");

    drop(fixture);
    let listing = || {
        let mut names: Vec<String> = std::fs::read_dir(&folder).unwrap().flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    };
    let before = listing();
    assert!(!before.iter().any(|name| name.ends_with("-wal")), "{before:?}");
    assert_eq!(chats(&read_home(&home)).count(), 2);
    assert_eq!(listing(), before, "reading created nothing");
}

#[test]
fn a_chat_without_a_chat_database_is_none_and_the_host_still_reads() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::at(dir.path(), 38, None);
    fixture.project("p1", Path::new("/code/shop"), "shop");
    let store = read_home(dir.path());
    assert_eq!((store.orgs[0].chat_version, store.orgs[0].chats.len()), (None, 0));
    assert_eq!(counts(&store).projects, 1);
}

#[test]
fn source_refs_round_trip() {
    let path = Path::new("/h/host/o/chat.db");
    assert_eq!(parse_source_ref(&source_ref(path, "c1")), Some((path.to_path_buf(), "c1".to_owned())));
    assert_eq!(parse_source_ref("no-hash"), None);
    assert_eq!(parse_source_ref("#x"), None);
    assert_eq!(parse_source_ref("/a#"), None);
}

#[test]
fn preview_copies_chats_and_lowers_configurations_to_inert_setup() {
    let sandbox = sandbox();
    sandbox.fixture.events("c-claude", "e1", &[user("u1", "Say hi"), agent("a1", "Hi!")]);
    let discovery = SupersetImporter
        .discover(&DiscoveryRequest {
            provider: PROVIDER.into(),
            approved_roots: vec![sandbox.home.to_string_lossy().into_owned()],
            selected_export: None,
            source_version: None,
            schema_version: None,
            format_versions: BTreeMap::new(),
        })
        .unwrap();
    assert_eq!(discovery.artifacts.len(), 4, "two chats and two configurations");
    let selection = DiscoverySelection { artifact_ids: discovery.artifacts.iter().map(|artifact| artifact.artifact_id.clone()).collect() };
    let candidates = SupersetImporter.preview(&discovery, &selection).unwrap();
    let kinds: Vec<CandidateKind> = candidates.iter().map(|candidate| candidate.kind.clone()).collect();
    assert_eq!(kinds.iter().filter(|kind| **kind == CandidateKind::Conversation).count(), 1, "{kinds:?}");
    assert_eq!(kinds.iter().filter(|kind| **kind == CandidateKind::Unsupported).count(), 1, "the chat with no copy");
    let configs: Vec<&ImportCandidate> = candidates.iter().filter(|candidate| candidate.kind == CandidateKind::ProviderInstance).collect();
    assert_eq!(configs.len(), 2);
    assert!(configs.iter().all(|candidate| candidate.normalized_payload["inert"] == json!(true)));
    assert!(!format!("{candidates:?}").contains(CANARY));
    let conversation = candidates.iter().find(|candidate| candidate.kind == CandidateKind::Conversation).unwrap().clone();
    assert_eq!(conversation.title, "Say hi", "Bridge's own title when Superset has none");
    assert_eq!(SupersetImporter.normalize(conversation.clone()).unwrap().candidate.source.provider, PROVIDER);
    let mut other = conversation.clone();
    other.source.provider = "orca".into();
    assert!(SupersetImporter.normalize(other.clone()).is_err());
    assert!(!SupersetImporter.validate(&other, &SchemaGate { format: STORE_GATE.into(), allowed_versions: vec![STORE_GATE.into()] }).accepted);
    let outside = tempfile::tempdir().unwrap();
    let mut stray = discovery.clone();
    stray.approved_roots = vec![outside.path().to_string_lossy().into_owned()];
    assert!(SupersetImporter.preview(&stray, &selection).unwrap().iter().all(|candidate| candidate.kind == CandidateKind::Unsupported));
    let mut wrong = discovery.clone();
    wrong.provider = "orca".into();
    assert!(SupersetImporter.preview(&wrong, &selection).is_err());
    assert_eq!(SupersetImporter.fixture_manifest().provider, PROVIDER);
}

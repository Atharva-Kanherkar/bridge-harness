use super::*;
use crate::import_worktrees::tests::{repo, worktree};
use crate::runtime::BridgeCore;
use serde_json::json;
use std::sync::Arc;

fn rules(data: &Path, home: &Path) -> Rules {
    Rules::new(data, home)
}

#[test]
fn slash_commands_are_told_apart_from_things_said() {
    for command in ["/usage", "/model sonnet", " /cost ", "/review-pr 12", "/mcp:list"] {
        assert!(is_slash_command(command), "{command:?}");
    }
    for said in ["", "/", "usage", "hi /usage", "/usr/bin/env", "/path/to/file.txt please", "/a\\nsecond line", "/fix\nthe login"] {
        assert!(!is_slash_command(said), "{said:?}");
    }
    assert!(!is_slash_command(&format!("/{}", "x".repeat(41))));
}

#[test]
fn a_run_of_only_slash_commands_is_a_probe_not_a_chat() {
    let user = |text: &str| json!({ "kind": "user.message", "text": text });
    assert!(is_command_run(&[user("/usage")]));
    assert!(is_command_run(&[user("/usage"), user("/cost")]));
    assert!(!is_command_run(&[]));
    assert!(!is_command_run(&[user("/review"), json!({ "kind": "assistant.message", "text": "Looks good." })]), "a command that got a reply is a chat");
    assert!(!is_command_run(&[user("/usage"), user("thanks")]));
    assert!(!is_command_run(&[user("/usage"), json!({ "kind": "tool.call" })]));
    assert!(!is_command_run(&[json!({ "kind": "user.message" })]));
    let output = "<local-command-stdout>Login successful</local-command-stdout>";
    assert!(is_command_run(&[user("/login"), user(output), user("/mcp"), user("<local-command-stdout>No MCP servers configured.</local-command-stdout>")]));
    assert!(!is_command_run(&[user(output)]), "output alone, with no command, is not a run");
    assert!(!is_command_run(&[user("/login"), user(output), user("now fix the build")]));
    assert!(is_command_output("  <local-command-stderr>oops</local-command-stderr>"));
    assert!(!is_command_output("<local-command-stdout is what I mean"));
}

#[test]
fn bridges_own_folders_are_recognised_by_path_and_by_claude_code_project_name() {
    let rules = rules(Path::new("/Users/me/Library/Application Support/dev.bridge.deck"), Path::new("/Users/me"));
    assert!(rules.is_bridge_folder("/Users/me/Library/Application Support/dev.bridge.deck/chats/3f2a"));
    assert!(rules.is_bridge_folder("/Users/me/Library/Application Support/dev.bridge.deck"));
    assert!(!rules.is_bridge_folder("/Users/me/Library/Application Support/dev.bridge.deck-other/x"));
    assert!(!rules.is_bridge_folder("relative/dev.bridge.deck/chats"));
    assert!(!rules.is_bridge_folder("/Users/me/code/app"));
    assert!(rules.is_bridge_claude_project("-Users-me-Library-Application-Support-dev-bridge-deck-chats-3f2a"));
    assert!(rules.is_bridge_claude_project("-Users-me-Library-Application-Support-dev-bridge-deck"));
    assert!(!rules.is_bridge_claude_project("-Users-me-Library-Application-Support-dev-bridge-deckhand"));
    assert!(!rules.is_bridge_claude_project("-Users-me-code-app"));
    assert_eq!(claude_slug(Path::new("/Users/me/a.b c")), "-Users-me-a-b-c");
}

#[test]
fn another_bridges_data_folder_is_known_by_its_layout() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().canonicalize().unwrap();
    let rules = rules(&root.join("this-bridge"), &root);
    let other = root.join("dev.bridge.deck.wt");
    std::fs::create_dir_all(other.join("chats")).unwrap();
    std::fs::write(other.join("bridge.db"), "").unwrap();
    assert!(rules.is_bridge_folder(&other.join("chats/3f2a").to_string_lossy()), "a chat's folder, even once gone");
    assert!(rules.is_bridge_folder(&other.join("worktrees/orchestrators/bridge/61de").to_string_lossy()));
    assert!(!rules.is_bridge_folder(&other.to_string_lossy()), "the data folder itself is not a run");
    // A project that happens to hold a bridge.db is still a project.
    let project = root.join("code/app");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(project.join("bridge.db"), "").unwrap();
    assert!(!rules.is_bridge_folder(&project.to_string_lossy()));
    assert!(!rules.is_bridge_folder(&project.join("src").to_string_lossy()));
    let lookalike = root.join("notes/chats");
    std::fs::create_dir_all(&lookalike).unwrap();
    assert!(!rules.is_bridge_folder(&lookalike.join("x").to_string_lossy()), "chats/ without a bridge.db beside it");
    // A Bridge data folder deleted altogether: its exact layout is enough.
    let gone = root.join("bench/data-xny7Pi");
    assert!(rules.is_bridge_folder(&gone.join("chats/8a166b66-bf0c-47ed-8856-d323a08b9ba2").to_string_lossy()));
    assert!(rules.is_bridge_folder(&gone.join("chats/title-runs").to_string_lossy()));
    assert!(rules.is_bridge_folder(&gone.join("worktrees/workers/w1").to_string_lossy()));
    assert!(!rules.is_bridge_folder(&gone.join("chats/my-notes").to_string_lossy()), "not a chat id");
    assert!(!rules.is_bridge_folder(&gone.join("worktrees/feature").to_string_lossy()));
    assert!(!rules.is_bridge_folder(&lookalike.join("8a166b66-bf0c-47ed-8856-d323a08b9ba2").to_string_lossy()), "an existing folder needs its bridge.db");
}

#[test]
fn background_candidates_are_bridges_own_runs_and_probes() {
    let rules = rules(Path::new("/data/bridge"), Path::new("/Users/me"));
    let payload = |cwd: &str, text: &str, reply: bool| {
        let mut messages = vec![json!({ "kind": "user.message", "text": text })];
        if reply {
            messages.push(json!({ "kind": "assistant.message", "text": "ok" }));
        }
        json!({ "cwd": cwd, "messages": messages })
    };
    assert!(is_background_candidate(&payload("/data/bridge/chats/1", "Slack inbox", true), &rules));
    assert!(is_background_candidate(&payload("/Users/me/.claude/rimo-usage", "/usage", false), &rules));
    assert!(!is_background_candidate(&payload("/Users/me/code/app", "fix the login", true), &rules));
    assert!(!is_background_candidate(&json!({}), &rules), "no folder and no messages is not a run");
}

#[test]
fn a_chat_belongs_to_its_repositorys_main_checkout() {
    let scratch = tempfile::tempdir().unwrap();
    let home = scratch.path().canonicalize().unwrap();
    let main = repo(&home.join("code"), "app");
    let rules = rules(&home.join("bridge-data"), &home);
    std::fs::create_dir_all(main.join("packages/web")).unwrap();
    assert_eq!(project_folder(&main.join("packages/web").to_string_lossy(), &rules), Some(main.clone()), "a subfolder");
    assert_eq!(project_folder(&main.to_string_lossy(), &rules), Some(main.clone()));
    let tree = worktree(&main, &home.join("worktrees/app-feature"), "feature");
    assert_eq!(project_folder(&tree.to_string_lossy(), &rules), Some(main.clone()), "a worktree");
    let claude_tree = format!("{}/.claude/worktrees/pr-77-review-9d57ec", main.display());
    assert_eq!(project_folder(&claude_tree, &rules), Some(main.clone()), "a deleted Claude Code worktree");
    let plain = home.join("notes");
    std::fs::create_dir_all(&plain).unwrap();
    assert_eq!(project_folder(&plain.to_string_lossy(), &rules), Some(plain.clone()), "a folder that is not a repository");
}

#[test]
fn folders_that_are_gone_temporary_bridges_home_or_hidden_are_no_project() {
    let scratch = tempfile::tempdir().unwrap();
    let home = scratch.path().canonicalize().unwrap();
    let data = home.join("bridge-data");
    std::fs::create_dir_all(data.join("chats/1")).unwrap();
    std::fs::create_dir_all(home.join(".claude/rimo-usage")).unwrap();
    std::fs::create_dir_all(home.join("tmp-like/run")).unwrap();
    let mut rules = rules(&data, &home);
    rules.temp_dirs = vec![home.join("tmp-like")];
    for cwd in [
        home.join("gone").to_string_lossy().into_owned(),
        format!("{}/gone-repo/.claude/worktrees/x", home.display()),
        data.join("chats/1").to_string_lossy().into_owned(),
        home.to_string_lossy().into_owned(),
        home.parent().unwrap().to_string_lossy().into_owned(),
        home.join(".claude/rimo-usage").to_string_lossy().into_owned(),
        home.join("tmp-like/run").to_string_lossy().into_owned(),
        "relative/folder".into(),
        "   ".into(),
    ] {
        assert_eq!(project_folder(&cwd, &rules), None, "{cwd}");
    }
}

fn core(scratch: &Path) -> Arc<BridgeCore> {
    Arc::new(BridgeCore::for_tests(scratch))
}

fn imported(core: &BridgeCore, id: &str, cwd: &str, parent: Option<&str>) {
    core.db
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind,cwd,parent_session_id) VALUES(?1,NULL,'claude','x','stopped','imported','imported',?2,?3)",
            params![id, cwd, parent],
        )
        .unwrap();
}

/// History as an import writes it: entries, then the provenance that makes
/// them immutable.
fn history(core: &BridgeCore, id: &str, entries: &[(&str, &str)]) {
    let db = core.db.lock().unwrap();
    for (index, (kind, text)) in entries.iter().enumerate() {
        db.execute(
            "INSERT INTO session_entries(id,session_id,sequence,kind,payload,created_at) VALUES(?1,?2,?3,?4,?5,'2026-11-01T00:00:00Z')",
            params![format!("{id}-{index}"), id, index as i64 + 1, kind, json!({ "kind": kind, "text": text }).to_string()],
        )
        .unwrap();
    }
    db.execute(
        "INSERT OR IGNORE INTO import_commits(id,importer_namespace,provider,plan,outcome,rollback_state,created_at) VALUES('i','bridge','claude_code','{}','{}','none','now')",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO session_heads(session_id,active_entry_id,updated_at) SELECT ?1, MAX(id), 'now' FROM session_entries WHERE session_id=?1 HAVING COUNT(*) > 0",
        params![id],
    )
    .unwrap();
    db.execute(
        "INSERT INTO imported_record_provenance(bridge_id,destination_kind,provider,adapter_version,canonical_source_ref,source_path_fingerprint,source_content_hash,imported_at,redaction_summary,stability,confidence_bps,import_id) VALUES(?1,'session','claude_code','1','/src','fp','hash','now','{}','stable',10000,'i')",
        params![id],
    )
    .unwrap();
    db.execute(
        "INSERT INTO import_native_sessions(native_key,session_id,content_hash,import_id,created_at) VALUES(?1,?2,'h','i','now')",
        params![format!("claude:{id}"), id],
    )
    .unwrap();
}

fn workspace_of(core: &BridgeCore, id: &str) -> Option<String> {
    core.db
        .lock()
        .unwrap()
        .query_row("SELECT w.path FROM sessions s JOIN workspaces w ON w.id=s.workspace_id WHERE s.id=?1", params![id], |row| row.get(0))
        .optional()
        .unwrap()
}

#[test]
fn imported_chats_are_filed_under_their_project_made_once_and_kept_where_they_were() {
    let scratch = tempfile::tempdir().unwrap();
    let home = scratch.path().canonicalize().unwrap();
    let core = core(&home.join("data"));
    let rules = Rules::for_core(&core);
    let app = repo(&home.join("code"), "app");
    let api = repo(&home.join("code"), "api");
    std::fs::create_dir_all(app.join("src")).unwrap();
    let tree = worktree(&app, &home.join("wt/app-x"), "x");
    // Bridge already has a project for api; app has none.
    core.db.lock().unwrap().execute(
        "INSERT INTO workspaces(id,title,status,path,created_at) VALUES('api-ws','api','idle',?1,'2026-01-01')",
        params![api.to_string_lossy()],
    ).unwrap();
    imported(&core, "in-app", &app.to_string_lossy(), None);
    imported(&core, "in-app-src", &app.join("src").to_string_lossy(), None);
    imported(&core, "in-tree", &tree.to_string_lossy(), None);
    imported(&core, "in-tree-child", &tree.to_string_lossy(), Some("in-tree"));
    imported(&core, "in-api", &api.to_string_lossy(), None);
    imported(&core, "gone", &home.join("gone").to_string_lossy(), None);
    imported(&core, "placed-already", &app.to_string_lossy(), None);
    core.db.lock().unwrap().execute("UPDATE sessions SET workspace_id='api-ws' WHERE id='placed-already'", []).unwrap();

    let placement = place(&core.db, &rules).unwrap();

    assert_eq!(placement.created, vec![app.clone()], "one project for app; api's is reused");
    assert_eq!(placement.placed, 5);
    let app_path = Some(app.to_string_lossy().into_owned());
    for id in ["in-app", "in-app-src", "in-tree", "in-tree-child"] {
        assert_eq!(workspace_of(&core, id), app_path, "{id}");
    }
    assert_eq!(workspace_of(&core, "in-api"), Some(api.to_string_lossy().into_owned()));
    assert_eq!(workspace_of(&core, "gone"), None, "a folder that is gone stays without a project");
    assert_eq!(workspace_of(&core, "placed-already"), Some(api.to_string_lossy().into_owned()), "a chat already filed is left alone");
    let (title, branch, project): (String, Option<String>, Option<String>) = core
        .db
        .lock()
        .unwrap()
        .query_row("SELECT title, branch, project_id FROM workspaces WHERE path=?1", params![app.to_string_lossy()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap();
    assert_eq!((title.as_str(), branch.as_deref()), ("app", Some("main")));
    assert!(project.is_some(), "a repository gets its project");

    assert_eq!(place(&core.db, &rules).unwrap(), Placement { placed: 0, created: Vec::new() }, "a second run changes nothing");
}

#[test]
fn a_chat_in_an_adopted_worktree_stays_with_that_workspace() {
    let scratch = tempfile::tempdir().unwrap();
    let home = scratch.path().canonicalize().unwrap();
    let core = core(&home.join("data"));
    let app = repo(&home.join("code"), "app");
    let tree = worktree(&app, &home.join("conductor/workspaces/app/tokyo"), "tokyo");
    core.db.lock().unwrap().execute(
        "INSERT INTO workspaces(id,title,status,path,created_at) VALUES('tokyo','Tokyo','idle',?1,'2026-01-01')",
        params![tree.to_string_lossy()],
    ).unwrap();
    imported(&core, "c", &tree.to_string_lossy(), None);
    let placement = place(&core.db, &Rules::for_core(&core)).unwrap();
    assert!(placement.created.is_empty());
    assert_eq!(workspace_of(&core, "c"), Some(tree.to_string_lossy().into_owned()));
}

#[test]
fn a_plain_folder_becomes_a_project_without_a_repository() {
    let scratch = tempfile::tempdir().unwrap();
    let home = scratch.path().canonicalize().unwrap();
    let core = core(&home.join("data"));
    let notes = home.join("notes");
    std::fs::create_dir_all(&notes).unwrap();
    imported(&core, "n", &notes.to_string_lossy(), None);
    place(&core.db, &Rules::for_core(&core)).unwrap();
    let project: Option<String> = core
        .db
        .lock()
        .unwrap()
        .query_row("SELECT project_id FROM workspaces WHERE path=?1", params![notes.to_string_lossy()], |row| row.get(0))
        .unwrap();
    assert_eq!(project, None);
    assert_eq!(workspace_of(&core, "n"), Some(notes.to_string_lossy().into_owned()));
}

#[test]
fn bridges_own_runs_and_probes_are_removed_and_real_chats_are_kept_immutable() {
    let scratch = tempfile::tempdir().unwrap();
    let home = scratch.path().canonicalize().unwrap();
    let data = home.join("data");
    let core = core(&data);
    let rules = Rules::for_core(&core);
    let slack = data.join("chats/0a1b").to_string_lossy().into_owned();
    imported(&core, "slack", &slack, None);
    history(&core, "slack", &[("user.message", "Check my Slack inbox"), ("assistant.message", "2 unread")]);
    imported(&core, "slack-child", &slack, Some("slack"));
    history(&core, "slack-child", &[("user.message", "read the channel")]);
    imported(&core, "probe", "/Users/me/.claude/rimo-usage", None);
    history(&core, "probe", &[("user.message", "/usage")]);
    imported(&core, "review", "/Users/me/code/app", None);
    history(&core, "review", &[("user.message", "/review"), ("assistant.message", "Looks good.")]);
    imported(&core, "chat", "/Users/me/code/app", None);
    history(&core, "chat", &[("user.message", "fix the login"), ("assistant.message", "Fixed.")]);
    imported(&core, "empty", "/Users/me/code/app", None);
    // A chat Bridge ran itself is never touched, even in its own folder.
    core.db.lock().unwrap().execute(
        "INSERT INTO sessions(id,harness,label,status,metric_source,kind,cwd) VALUES('own','claude','x','stopped','reported','chat',?1)",
        params![slack],
    ).unwrap();

    assert_eq!(purge_background(&core.db, &rules).unwrap().removed, 2);

    let db = core.db.lock().unwrap();
    let left: Vec<String> = db
        .prepare("SELECT id FROM sessions ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(left, vec!["chat", "empty", "own", "review"]);
    let orphans: i64 = db
        .query_row("SELECT COUNT(*) FROM session_entries WHERE session_id IN ('slack','slack-child','probe')", [], |row| row.get(0))
        .unwrap();
    assert_eq!(orphans, 0, "their history went with them");
    let natives: i64 = db.query_row("SELECT COUNT(*) FROM import_native_sessions", [], |row| row.get(0)).unwrap();
    assert_eq!(natives, 2, "only the kept chats' native ids remain");
    let provenance: i64 = db.query_row("SELECT COUNT(*) FROM imported_record_provenance", [], |row| row.get(0)).unwrap();
    assert_eq!(provenance, 5, "provenance stays as the audit of what was imported");
    let refused = db.execute("DELETE FROM session_entries WHERE session_id='chat'", []).unwrap_err();
    assert!(refused.to_string().contains("immutable"), "the guard is back: {refused}");
    drop(db);
    assert_eq!(purge_background(&core.db, &rules).unwrap().removed, 0, "nothing left to remove");
}

#[test]
fn removal_goes_in_batches() {
    let scratch = tempfile::tempdir().unwrap();
    let home = scratch.path().canonicalize().unwrap();
    let data = home.join("data");
    let core = core(&data);
    for index in 0..(PURGE_BATCH + 7) {
        let id = format!("run-{index}");
        imported(&core, &id, &data.join(format!("chats/{index}")).to_string_lossy(), None);
        history(&core, &id, &[("user.message", "Slack inbox")]);
    }
    assert_eq!(purge_background(&core.db, &Rules::for_core(&core)).unwrap().removed, PURGE_BATCH + 7);
    let left: i64 = core.db.lock().unwrap().query_row("SELECT COUNT(*) FROM sessions WHERE kind='imported'", [], |row| row.get(0)).unwrap();
    assert_eq!(left, 0);
}

/// What tidying would do to this Mac's real Bridge data, read-only: how many
/// imported runs it would remove and which projects the rest would land in.
/// Writes nothing.
#[test]
#[ignore = "reads the real Bridge database; run by hand to measure"]
fn measure_tidying_on_this_machine() {
    let home = PathBuf::from(std::env::var("HOME").unwrap());
    let data = home.join("Library/Application Support/dev.bridge.deck");
    let uri = format!("file:{}?mode=ro", crate::import_sqlite::uri_path(&data.join("bridge.db")));
    let db = Connection::open_with_flags(uri, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI).unwrap();
    let mut rules = Rules::new(&data, &home);
    rules.temp_dirs = os_temp_dirs();
    let started = std::time::Instant::now();
    let db = Mutex::new(db);
    let doomed = background_sessions(&db, &rules, &mut std::time::Duration::ZERO).unwrap();
    let db = db.into_inner().unwrap();
    let total: i64 = db.query_row("SELECT COUNT(*) FROM sessions WHERE kind='imported' AND parent_session_id IS NULL", [], |row| row.get(0)).unwrap();
    println!("MEASURE imported={total} would_remove={} in {:.1}s", doomed.len(), started.elapsed().as_secs_f64());
    let doomed: std::collections::HashSet<String> = doomed.into_iter().collect();
    let mut statement = db.prepare("SELECT id, cwd, workspace_id FROM sessions WHERE kind='imported' AND parent_session_id IS NULL").unwrap();
    let mut by_project: BTreeMap<String, usize> = BTreeMap::new();
    let mut resolved: HashMap<String, Option<PathBuf>> = HashMap::new();
    let (mut kept, mut already, mut placed, mut unplaced) = (0, 0, 0, 0);
    let mut left: BTreeMap<String, usize> = BTreeMap::new();
    for row in statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<String>>(2)?))).unwrap() {
        let (id, cwd, workspace) = row.unwrap();
        if doomed.contains(&id) {
            continue;
        }
        kept += 1;
        if workspace.is_some() {
            already += 1;
            continue;
        }
        let cwd = cwd.unwrap_or_default();
        let folder = resolved.entry(cwd.clone()).or_insert_with(|| project_folder(&cwd, &rules)).clone();
        match folder {
            Some(folder) => {
                placed += 1;
                *by_project.entry(folder.to_string_lossy().replace(&*home.to_string_lossy(), "~")).or_insert(0) += 1;
            }
            None => {
                unplaced += 1;
                *left.entry(cwd.replace(&*home.to_string_lossy(), "~")).or_insert(0) += 1;
            }
        }
    }
    println!("MEASURE kept={kept} already_in_a_project={already} would_place={placed} stay_without_project={unplaced} projects={}", by_project.len());
    for (project, count) in &by_project {
        println!("MEASURE project {count:5} {project}");
    }
    let mut left: Vec<(usize, String)> = left.into_iter().map(|(cwd, count)| (count, cwd)).collect();
    left.sort_by(|a, b| b.cmp(a));
    for (count, cwd) in left.iter().take(25) {
        println!("MEASURE unplaced {count:5} {cwd} exists={}", Path::new(&cwd.replacen('~', &home.to_string_lossy(), 1)).exists());
    }
}

#[test]
fn adopting_a_worktree_later_moves_its_chats_from_the_repository_to_it() {
    let scratch = tempfile::tempdir().unwrap();
    let home = scratch.path().canonicalize().unwrap();
    let core = core(&home.join("data"));
    let app = repo(&home.join("code"), "app");
    let tree = worktree(&app, &home.join("conductor/workspaces/app/tokyo"), "tokyo");
    imported(&core, "c", &tree.to_string_lossy(), None);
    imported(&core, "r", &app.to_string_lossy(), None);
    place(&core.db, &Rules::for_core(&core)).unwrap();
    assert_eq!(workspace_of(&core, "c"), Some(app.to_string_lossy().into_owned()), "first under its repository");
    let candidate = crate::import_worktrees::tests::candidate(&tree);
    let report = crate::import_worktrees::adopt(&core.db, vec![candidate]).unwrap();
    assert_eq!(report.relinked_chats, 1);
    assert_eq!(workspace_of(&core, "c"), Some(tree.to_string_lossy().into_owned()), "then under its own worktree");
    assert_eq!(workspace_of(&core, "r"), Some(app.to_string_lossy().into_owned()), "the repository's own chat stays");
}

/// The real purge on a copy of a real Bridge database
/// (`BRIDGE_PURGE_COPY=<copy>/bridge.db`), opened the way Bridge opens it:
/// how long it takes and the longest it holds the database.
#[test]
#[ignore = "deletes from the database it is given; run by hand on a copy"]
fn measure_the_purge_on_a_copy() {
    let path = PathBuf::from(std::env::var("BRIDGE_PURGE_COPY").expect("set BRIDGE_PURGE_COPY to a copy of bridge.db"));
    let db = Connection::open(&path).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;").unwrap();
    let home = PathBuf::from(std::env::var("HOME").unwrap());
    let mut rules = Rules::new(&home.join("Library/Application Support/dev.bridge.deck"), &home);
    rules.temp_dirs = os_temp_dirs();
    let db = Mutex::new(db);
    let before: i64 = db.lock().unwrap().query_row("SELECT COUNT(*) FROM sessions WHERE kind='imported'", [], |row| row.get(0)).unwrap();
    let started = std::time::Instant::now();
    let purge = purge_background(&db, &rules).unwrap();
    let total = started.elapsed();
    let db = db.into_inner().unwrap();
    let after: i64 = db.query_row("SELECT COUNT(*) FROM sessions WHERE kind='imported'", [], |row| row.get(0)).unwrap();
    let foreign_keys: i64 = db.query_row("PRAGMA foreign_keys", [], |row| row.get(0)).unwrap();
    let guard: i64 = db.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='trigger' AND name IN ('imported_session_entries_immutable_delete','session_entries_ad_fts')", [], |row| row.get(0)).unwrap();
    let orphan_entries: i64 = db.query_row("SELECT COUNT(*) FROM session_entries e WHERE NOT EXISTS (SELECT 1 FROM sessions s WHERE s.id=e.session_id)", [], |row| row.get(0)).unwrap();
    let orphan_heads: i64 = db.query_row("SELECT COUNT(*) FROM session_heads h WHERE NOT EXISTS (SELECT 1 FROM sessions s WHERE s.id=h.session_id)", [], |row| row.get(0)).unwrap();
    println!(
        "MEASURE removed={} batches={} total_secs={:.1} longest_hold_ms={} imported_before={before} imported_after={after} foreign_keys={foreign_keys} triggers_back={guard} orphan_entries={orphan_entries} orphan_heads={orphan_heads}",
        purge.removed,
        purge.batches,
        total.as_secs_f64(),
        purge.longest_hold.as_millis()
    );
}

fn search_rows(core: &BridgeCore, session: &str) -> i64 {
    core.db
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM session_entry_fts WHERE session_id=?1", params![session], |row| row.get(0))
        .unwrap()
}

#[test]
fn a_removed_run_leaves_no_search_rows_digest_or_lifted_guard_and_checks_come_back_on() {
    let scratch = tempfile::tempdir().unwrap();
    let home = scratch.path().canonicalize().unwrap();
    let data = home.join("data");
    let core = core(&data);
    let slack = data.join("chats/0a1b").to_string_lossy().into_owned();
    imported(&core, "slack", &slack, None);
    history(&core, "slack", &[("user.message", "Check my Slack inbox"), ("assistant.message", "2 unread")]);
    imported(&core, "chat", "/Users/me/code/app", None);
    history(&core, "chat", &[("user.message", "fix the login"), ("assistant.message", "Fixed.")]);
    assert_eq!((search_rows(&core, "slack"), search_rows(&core, "chat")), (2, 2), "both are in the search index");

    let purge = purge_background(&core.db, &Rules::for_core(&core)).unwrap();

    assert_eq!((purge.removed, purge.batches), (1, 1));
    assert_eq!(search_rows(&core, "slack"), 0, "the run's search rows went with it");
    assert_eq!(search_rows(&core, "chat"), 2, "the kept chat is still searchable");
    let db = core.db.lock().unwrap();
    let digests: i64 = db.query_row("SELECT COUNT(*) FROM chat_digests WHERE session_id='slack'", [], |row| row.get(0)).unwrap();
    assert_eq!(digests, 0);
    let foreign_keys: i64 = db.query_row("PRAGMA foreign_keys", [], |row| row.get(0)).unwrap();
    assert_eq!(foreign_keys, 1, "checks are back on");
    let triggers: Vec<String> = db
        .prepare("SELECT name FROM sqlite_master WHERE type='trigger' AND name IN ('imported_session_entries_immutable_delete','session_entries_ad_fts') ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(triggers, vec!["imported_session_entries_immutable_delete", "session_entries_ad_fts"]);
}

#[test]
fn a_run_something_else_points_at_is_removed_with_checks_on() {
    let scratch = tempfile::tempdir().unwrap();
    let home = scratch.path().canonicalize().unwrap();
    let data = home.join("data");
    let core = core(&data);
    let slack = data.join("chats/0a1b").to_string_lossy().into_owned();
    imported(&core, "slack", &slack, None);
    history(&core, "slack", &[("user.message", "Check my Slack inbox")]);
    // A title job points at it: its foreign key cascades.
    core.db
        .lock()
        .unwrap()
        .execute("INSERT INTO session_title_jobs(session_id,status,reason,available_at,created_at,updated_at) VALUES('slack','pending','backfill','now','now','now')", [])
        .map(|_| ())
        .unwrap_or_else(|error| panic!("the title job fixture needs updating: {error}"));
    let purge = purge_background(&core.db, &Rules::for_core(&core)).unwrap();
    assert_eq!(purge.removed, 1);
    let db = core.db.lock().unwrap();
    let left: i64 = db.query_row("SELECT COUNT(*) FROM session_title_jobs WHERE session_id='slack'", [], |row| row.get(0)).unwrap();
    assert_eq!(left, 0, "the cascade was carried out");
    let foreign_keys: i64 = db.query_row("PRAGMA foreign_keys", [], |row| row.get(0)).unwrap();
    assert_eq!(foreign_keys, 1);
}

/// The regression: with foreign key checks on, every deleted entry made
/// SQLite read the whole history (`parent_entry_id` has no index). On a large
/// history the old purge took most of a second per entry and held Bridge's
/// database for minutes per batch.
#[test]
fn removal_stays_fast_on_a_large_history() {
    let scratch = tempfile::tempdir().unwrap();
    let home = scratch.path().canonicalize().unwrap();
    let data = home.join("data");
    let core = core(&data);
    {
        let db = core.db.lock().unwrap();
        db.execute(
            "INSERT INTO sessions(id,harness,label,status,metric_source,kind,cwd) VALUES('big','claude','x','stopped','reported','chat','/Users/me/code/app')",
            [],
        )
        .unwrap();
        // 400,000 entries of a chat Bridge ran, each pointing at the one before.
        db.execute_batch(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 400000)
             INSERT INTO session_entries(id,session_id,parent_entry_id,sequence,kind,payload,created_at)
             SELECT 'big-' || i, 'big', CASE WHEN i > 1 THEN 'big-' || (i - 1) END, i, 'tool.call', '{}', 'now' FROM n;",
        )
        .unwrap();
    }
    for index in 0..300 {
        let id = format!("run-{index}");
        imported(&core, &id, &data.join(format!("chats/{index}")).to_string_lossy(), None);
        history(&core, &id, &[("user.message", "Slack inbox"), ("assistant.message", "nothing new")]);
    }
    let started = std::time::Instant::now();
    let purge = purge_background(&core.db, &Rules::for_core(&core)).unwrap();
    let total = started.elapsed();
    println!("purge held the database at most {:?}, {total:?} in all", purge.longest_hold);
    assert_eq!(purge.removed, 300);
    assert!(purge.longest_hold < std::time::Duration::from_millis(1500), "held the database for {:?}", purge.longest_hold);
    assert!(total < std::time::Duration::from_secs(10), "took {total:?}");
    let kept: i64 = core.db.lock().unwrap().query_row("SELECT COUNT(*) FROM session_entries WHERE session_id='big'", [], |row| row.get(0)).unwrap();
    assert_eq!(kept, 400_000);
}

use super::*;
use crate::store;
use serde_json::json;
use bridge_protocol::messages::ChatToolKind::{AllSkills, App, Apps, Plugin, Server, Skill};
use bridge_protocol::messages::ChatToolState::{Fixed, Off, On};
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::Arc;

const CHAT: LaunchTarget<'static> = LaunchTarget { harness: "claude", interactive: true, workflow: false };

fn db() -> Connection {
    store::open(std::path::Path::new(":memory:")).unwrap()
}

fn chat(db: &Connection, id: &str, harness: &str, account: Option<&str>) {
    db.execute(
        "INSERT INTO sessions(id,harness,label,status,metric_source,kind,account) VALUES(?1,?2,'Chat','ready','reported','direct',?3)",
        params![id, harness, account],
    )
    .unwrap();
}

fn choice(value: Value) -> ChatToolSelection {
    serde_json::from_value(value).unwrap()
}

fn message<T: std::fmt::Debug>(result: Result<T, BridgeError>) -> String {
    match result {
        Err(BridgeError::Invalid(message)) => message,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn refused(value: Value) -> String {
    message(check(&choice(value)))
}

fn row(value: Value) -> ToolSelectionRow {
    ToolSelectionRow::new(1, check(&choice(value)).unwrap())
}

fn item<'a>(items: &'a [ChatToolItem], kind: ChatToolKind, id: &str) -> &'a ChatToolItem {
    items
        .iter()
        .find(|item| item.kind == kind && item.id == id)
        .unwrap_or_else(|| panic!("no {kind:?} {id} in {items:#?}"))
}

fn names(prefix: &str, count: usize) -> Vec<String> {
    (0..count).map(|i| format!("{prefix}{i:0>127}")).collect()
}

fn connector(server: &str, label: &str, harness: &str) -> ConnectorDescriptor {
    ConnectorDescriptor {
        family: server.to_owned(),
        display_name: label.to_owned(),
        server: Some(server.to_owned()),
        harness: Some(harness.to_owned()),
        has_inbox: false,
        available: true,
        reason: None,
        explanation: None,
    }
}

/// The reply that the app-control path gives for a raw Codex result.
fn via(kind: &str, raw: Value) -> Value {
    let reply = crate::apps::codex::translate(&json!({"id": format!("bridge-app:{kind}:t"), "result": raw})).unwrap();
    assert_eq!(reply["ok"], true, "{reply}");
    reply["result"].clone()
}

fn codex_status(servers: Value) -> CodexSource {
    CodexSource {
        status: Some(via("servers", json!({ "data": servers }))),
        config: Some(via("config", json!({"config": {}}))),
        ..CodexSource::default()
    }
}

#[test]
fn a_chat_without_a_row_launches_with_no_tool_launch() {
    let db = db();
    chat(&db, "c", "claude", None);
    assert_eq!(read(&db, "c").unwrap(), None);
    assert_eq!(launch_for(&db, "c", &CHAT).unwrap(), None);

    let saved = choice(json!({"harness": "claude", "serversOff": ["github"]}));
    write_checked(&db, "c", 0, &saved).unwrap();
    let launch = launch_for(&db, "c", &CHAT).unwrap().unwrap();
    assert_eq!((launch.revision, launch.servers_off), (1, vec!["github".to_owned()]));

    // Only an interactive top-level chat outside a workflow reads the row.
    assert_eq!(launch_for(&db, "c", &LaunchTarget { interactive: false, ..CHAT }).unwrap(), None);
    assert_eq!(launch_for(&db, "c", &LaunchTarget { workflow: true, ..CHAT }).unwrap(), None);
    db.execute("INSERT INTO sessions(id,harness,label,status,metric_source,parent_session_id) VALUES('child','claude','Worker','working','reported','c')", []).unwrap();
    write_checked(&db, "child", 0, &saved).unwrap();
    assert_eq!(launch_for(&db, "child", &CHAT).unwrap(), None);
    chat(&db, "worker", "claude", None);
    db.execute("INSERT INTO worker_runtime(session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,result_status,updated_at) VALUES('worker','c','working','research','key','pending','now')", []).unwrap();
    write_checked(&db, "worker", 0, &saved).unwrap();
    assert_eq!(launch_for(&db, "worker", &CHAT).unwrap(), None);

    // A launch that is not a top-level chat never reads the row, so a corrupt
    // row cannot fail it.
    let corrupt = json!({"revision": 1, "harness": "claude", "account": null, "serversOff": ["bridge"]});
    chat_defaults::write(&db, KIND, "worker", &corrupt).unwrap();
    chat_defaults::write(&db, KIND, "child", &corrupt).unwrap();
    assert_eq!(launch_for(&db, "worker", &CHAT).unwrap(), None);
    assert_eq!(launch_for(&db, "child", &CHAT).unwrap(), None);
    chat_defaults::write(&db, KIND, "c", &corrupt).unwrap();
    assert_eq!(launch_for(&db, "c", &LaunchTarget { workflow: true, ..CHAT }).unwrap(), None);
    assert!(launch_for(&db, "c", &CHAT).is_err());
}

#[test]
fn the_off_list_keeps_a_new_connector_on() {
    let saved = row(json!({"harness": "claude", "serversOff": ["github"]}));
    let source = ClaudeSource { servers: vec!["github".into(), "linear".into()], ..ClaudeSource::default() };
    let items = claude_units(&source, Some(&saved));
    assert_eq!(item(&items, Server, "github").state, Off);
    assert_eq!(item(&items, Server, "linear").state, On);
    assert_eq!(ToolLaunch::from_row(&saved).servers_off, ["github"]);

    let saved = row(json!({"harness": "codex", "serversOff": ["github"]}));
    let source = codex_status(json!([{"name": "github", "tools": {}}, {"name": "sentry", "tools": {}}]));
    let items = codex_units(&source, Some(&saved));
    assert_eq!(item(&items, Server, "github").state, Off);
    assert_eq!(item(&items, Server, "sentry").state, On);

    let saved = row(json!({"harness": "opencode", "serversOff": ["github"]}));
    let source = OpenCodeSource { mcp: Some(json!({"github": {"status": "connected"}, "sentry": {"status": "connected"}})) };
    let items = opencode_units(&source, Some(&saved));
    assert_eq!(item(&items, Server, "github").state, Off);
    assert_eq!(item(&items, Server, "sentry").state, On);
    assert_eq!(ToolLaunch::from_row(&saved).servers_off, ["github"]);
}

#[test]
fn the_claude_skill_allowlist_keeps_an_empty_set_distinct_from_all() {
    let db = db();
    chat(&db, "all", "claude", None);
    chat(&db, "none", "claude", None);
    write_checked(&db, "all", 0, &choice(json!({"harness": "claude", "claudeSkills": null}))).unwrap();
    write_checked(&db, "none", 0, &choice(json!({"harness": "claude", "claudeSkills": {"only": []}}))).unwrap();
    let payload = |id: &str| -> Value {
        let text: String = db
            .query_row("SELECT payload FROM configuration_entries WHERE kind=?1 AND id=?2", params![KIND, id], |r| r.get(0))
            .unwrap();
        serde_json::from_str(&text).unwrap()
    };
    assert_eq!(payload("all")["claudeSkills"], Value::Null);
    assert_eq!(payload("none")["claudeSkills"], json!({"only": []}));

    let all = launch_for(&db, "all", &CHAT).unwrap().unwrap();
    let none = launch_for(&db, "none", &CHAT).unwrap().unwrap();
    assert_eq!(all.claude_skills, None);
    assert_eq!(none.claude_skills, Some(Vec::new()));
    assert_ne!(all.effective_hash, none.effective_hash);
    assert_eq!(all.effective_hash, all_on_hash("claude"));

    let source = ClaudeSource { skills: vec!["review".into()], ..ClaudeSource::default() };
    let all_items = claude_units(&source, read(&db, "all").unwrap().as_ref());
    let none_items = claude_units(&source, read(&db, "none").unwrap().as_ref());
    assert_eq!(item(&all_items, Skill, "review").state, On);
    assert_eq!(item(&none_items, Skill, "review").state, Off);
}

#[test]
fn a_new_claude_skill_shows_as_left_out() {
    let saved = row(json!({"harness": "claude", "claudeSkills": {"only": ["review"]}}));
    let source = ClaudeSource { skills: vec!["review".into(), "deploy".into()], ..ClaudeSource::default() };
    let items = claude_units(&source, Some(&saved));
    assert_eq!(item(&items, Skill, "review").state, On);
    let deploy = item(&items, Skill, "deploy");
    assert_eq!((deploy.state, deploy.seen), (Off, true));
    // The launch keeps only the listed skills, so the new skill does not load.
    assert_eq!(ToolLaunch::from_row(&saved).claude_skills, Some(vec!["review".to_owned()]));
}

#[test]
fn an_omission_of_bridge_is_refused() {
    for harness in ["claude", "codex", "opencode"] {
        for name in ["bridge", "Bridge", " bridge "] {
            let refusal = refused(json!({"harness": harness, "serversOff": [name, "github"]}));
            assert!(refusal.contains("Bridge server"), "{harness} {name:?}: {refusal}");
        }
    }
    let db = db();
    chat(&db, "c", "claude", None);
    let refusal = message(write_checked(&db, "c", 0, &choice(json!({"harness": "claude", "serversOff": ["bridge"]}))));
    assert!(refusal.contains("Bridge server"), "{refusal}");
    assert_eq!(read(&db, "c").unwrap(), None);

    // A row that holds the Bridge server by other means does not read, and a
    // launch never leaves the server out.
    chat_defaults::write(&db, KIND, "c", &json!({"revision": 1, "harness": "claude", "account": null, "serversOff": ["bridge"]})).unwrap();
    assert!(message(launch_for(&db, "c", &CHAT)).contains("Choose its tools again"));
    let forged = ToolSelectionRow { servers_off: vec!["bridge".into(), "github".into()], ..row(json!({"harness": "claude"})) };
    assert_eq!(ToolLaunch::from_row(&forged).servers_off, ["github"]);
}

#[test]
fn ids_with_control_characters_duplicates_or_too_many_bytes_are_refused() {
    let cases = [
        json!({"harness": "claude", "serversOff": ["git\nhub"]}),
        json!({"harness": "claude", "serversOff": [""]}),
        json!({"harness": "claude", "serversOff": ["a".repeat(129)]}),
        json!({"harness": "claude", "serversOff": ["é".repeat(65)]}),
        json!({"harness": "claude", "serversOff": ["github", "github"]}),
        json!({"harness": "claude", "serversOff": (0..129).map(|i| format!("s{i}")).collect::<Vec<_>>()}),
        json!({"harness": "claude", "claudeSkills": {"only": ["re\u{7}view"]}}),
        json!({"harness": "claude", "claudeSkills": {"only": ["review", "review"]}}),
        json!({"harness": "claude", "claudeSkills": {"only": (0..385).map(|i| format!("k{i}")).collect::<Vec<_>>()}}),
        json!({"harness": "codex", "codexSkillsOff": ["x\u{0}"]}),
        json!({"harness": "codex", "codexSkillsOff": ["s", "s"]}),
        json!({"harness": "claude", "account": "a\tb"}),
        json!({"harness": "claude", "account": "a".repeat(129)}),
        json!({"harness": "gemini"}),
        json!({"harness": "claude", "codexAppsOff": true}),
        json!({"harness": "codex", "claudeSkills": {"only": []}}),
        json!({"harness": "opencode", "codexSkillsOff": ["s"]}),
    ];
    for case in cases {
        assert!(!refused(case.clone()).is_empty(), "{case}");
    }
    assert!(serde_json::from_value::<ChatToolSelection>(json!({"harness": "claude", "extra": 1})).is_err());
    let longest = check(&choice(json!({"harness": "claude", "serversOff": ["a".repeat(128), "é".repeat(64)]}))).unwrap();
    assert_eq!(longest.servers_off.len(), 2);
    let most = check(&choice(json!({"harness": "claude", "serversOff": (0..128).map(|i| format!("s{i}")).collect::<Vec<_>>()}))).unwrap();
    assert_eq!(most.servers_off.len(), 128);
}

#[test]
fn claude_skill_names_that_the_sdk_rejects_are_refused() {
    for name in ["", " ", " review", "review ", "a(b", "a)b", "a,b", "a\u{1}b", "*", "plugin:*", "review *", "/review", "a\\\\b", "review\\"] {
        assert!(!claude_sdk_accepts(name), "{name:?}");
        let message = refused(json!({"harness": "claude", "claudeSkills": {"only": ["review", name]}}));
        assert!(!message.is_empty(), "{name:?}");
    }
    for name in ["review", "chrome-devtools-mcp:a11y-debugging", "a b", "a\\b", "a*b", "superpowers:brainstorming"] {
        assert!(claude_sdk_accepts(name), "{name:?}");
        check(&choice(json!({"harness": "claude", "claudeSkills": {"only": [name]}}))).unwrap();
    }
}

#[test]
fn a_codex_key_outside_the_safe_set_is_refused() {
    let (longest, too_long) = ("a".repeat(64), "a".repeat(65));
    for key in ["my.server", "my server", "a\"b", "a=b", "ü", "a/b", too_long.as_str()] {
        assert!(!codex_key_is_safe(key), "{key:?}");
        let refusal = refused(json!({"harness": "codex", "serversOff": [key]}));
        assert!(refusal.contains("safe set"), "{key:?}: {refusal}");
    }
    for key in ["github", "my_server-2", "A9", longest.as_str()] {
        assert!(codex_key_is_safe(key), "{key:?}");
        check(&choice(json!({"harness": "codex", "serversOff": [key]}))).unwrap();
    }
    assert!(refused(json!({"harness": "codex", "serversOff": ["codex_apps"]})).contains("Apps switch"));
    assert!(!refused(json!({"harness": "codex", "codexSkillsOff": ["a\"b"]})).is_empty());
    assert!(!refused(json!({"harness": "codex", "codexSkillsOff": ["a\\b"]})).is_empty());
    // The safe set is a Codex rule. A Claude server keeps its display name.
    check(&choice(json!({"harness": "claude", "serversOff": ["claude.ai Gmail"]}))).unwrap();
}

#[test]
fn the_serialized_row_stays_within_half_the_reader_cap() {
    let db = db();
    chat(&db, "c", "claude", None);
    let full = choice(json!({"harness": "claude", "serversOff": names("s", 128), "claudeSkills": {"only": names("k", 384)}}));
    check(&full).unwrap();
    let refusal = message(write_checked(&db, "c", 0, &full));
    assert!(refusal.contains("limit"), "{refusal}");
    assert_eq!(read(&db, "c").unwrap(), None);

    let fits = choice(json!({"harness": "claude", "serversOff": names("s", 100), "claudeSkills": {"only": names("k", 300)}}));
    let stored = write_checked(&db, "c", 0, &fits).unwrap();
    let text: String = db
        .query_row("SELECT payload FROM configuration_entries WHERE kind=?1 AND id='c'", [KIND], |r| r.get(0))
        .unwrap();
    assert!(text.len() <= MAX_ROW_BYTES && MAX_ROW_BYTES * 2 == 128 * 1024, "{}", text.len());
    assert_eq!(read(&db, "c").unwrap(), Some(stored));
}

#[test]
fn a_stale_revision_is_refused() {
    let db = db();
    chat(&db, "c", "codex", None);
    let first = choice(json!({"harness": "codex", "serversOff": ["github"]}));
    let second = choice(json!({"harness": "codex", "codexAppsOff": true}));
    assert_eq!(write_checked(&db, "c", 0, &first).unwrap().revision, 1);
    assert!(message(write_checked(&db, "c", 0, &second)).contains("another window"));
    assert!(message(write_checked(&db, "c", 2, &second)).contains("another window"));
    assert_eq!(read(&db, "c").unwrap().unwrap().choice(), check(&first).unwrap());
    let saved = write_checked(&db, "c", 1, &second).unwrap();
    assert_eq!((saved.revision, saved.codex_apps_off), (2, true));

    // A row that does not read counts as revision 0, so a save can replace it.
    chat_defaults::write(&db, KIND, "c", &json!({"broken": true})).unwrap();
    assert!(read(&db, "c").is_err());
    assert_eq!(write_checked(&db, "c", 0, &first).unwrap().revision, 1);
}

#[test]
fn a_choice_for_another_harness_or_account_is_kept_and_not_applied() {
    let db = db();
    chat(&db, "c", "claude", Some("work"));
    write_checked(&db, "c", 0, &choice(json!({"harness": "claude", "account": "work", "serversOff": ["github"]}))).unwrap();
    assert!(launch_for(&db, "c", &CHAT).unwrap().is_some());

    db.execute("UPDATE sessions SET account=NULL WHERE id='c'", []).unwrap();
    assert_eq!(launch_for(&db, "c", &CHAT).unwrap(), None);
    let stored = read(&db, "c").unwrap().expect("the row stays");
    let label = status(&stored, "claude", None);
    assert!(!label.applies);
    assert_eq!(label.not_applied_reason.as_deref(), Some("Chosen for Claude Code (account work), not applied"));

    db.execute("UPDATE sessions SET account='work' WHERE id='c'", []).unwrap();
    assert_eq!(launch_for(&db, "c", &LaunchTarget { harness: "codex", ..CHAT }).unwrap(), None);
    assert!(!status(&stored, "codex", Some("work")).applies);
    let applied = status(&stored, "claude", Some(" work "));
    assert!(applied.applies && applied.not_applied_reason.is_none());

    // The harness id and a blank account mean the own login of the user.
    let own = row(json!({"harness": "codex", "account": "codex"}));
    assert_eq!(own.account, None);
    assert!(status(&own, "codex", None).applies && status(&own, "codex", Some(" ")).applies);

    let model = ChatModelChoice { harness: "codex".into(), model: None, account: None };
    let refusal = message(write_first(&db, "d", &model, &choice(json!({"harness": "claude"}))));
    assert!(refusal.contains("made for Claude Code (own login), but the chat starts on Codex (own login)"), "{refusal}");
    let refusal = message(write_first(&db, "d", &model, &choice(json!({"harness": "codex", "account": "work"}))));
    assert!(refusal.contains("Codex (account work)"), "{refusal}");
    assert_eq!(read(&db, "d").unwrap(), None);
}

#[test]
fn a_saved_name_that_the_source_does_not_list_stays_visible() {
    let saved = row(json!({"harness": "claude", "serversOff": ["gone"], "claudeSkills": {"only": ["old-skill", "kept"]}}));
    let source = ClaudeSource { servers: vec!["github".into()], skills: vec!["kept".into()], ..ClaudeSource::default() };
    let items = claude_units(&source, Some(&saved));
    let gone = item(&items, Server, "gone");
    assert_eq!((gone.state, gone.seen), (Off, false));
    let old = item(&items, Skill, "old-skill");
    assert_eq!((old.state, old.seen), (On, false));
    assert!(item(&items, Server, "github").seen && item(&items, Skill, "kept").seen);

    let saved = row(json!({"harness": "codex", "serversOff": ["gone"], "codexSkillsOff": ["old"]}));
    let items = codex_units(&codex_status(json!([])), Some(&saved));
    let gone = item(&items, Server, "gone");
    assert_eq!((gone.state, gone.seen), (Off, false));
    let old = item(&items, Skill, "old");
    assert_eq!((old.state, old.seen), (Off, false));

    let saved = row(json!({"harness": "opencode", "serversOff": ["gone"]}));
    let items = opencode_units(&OpenCodeSource::default(), Some(&saved));
    let gone = item(&items, Server, "gone");
    assert_eq!((gone.state, gone.seen), (Off, false));
}

#[test]
fn claude_units_are_each_server_and_each_skill() {
    let init = json!({
        "mcp_servers": [
            {"name": "bridge", "status": "connected"},
            {"name": "github", "status": "connected"},
            {"name": "claude.ai Gmail", "status": "connected"},
            {"name": "plugin:chrome-devtools-mcp:chrome-devtools", "status": "connected"}
        ],
        "skills": ["review", "chrome-devtools-mcp:a11y-debugging"],
        "plugins": [{"name": "chrome-devtools-mcp", "path": "/p"}]
    });
    let mut source = ClaudeSource::from_init(&init);
    source.catalog = vec![connector("linear", "Linear", "claude"), connector("figma", "Figma", "codex")];
    source.figures = Some(TokenFigures {
        source: "claude.getContextUsage".into(),
        at: Some("2026-11-19T10:00:00Z".into()),
        servers: BTreeMap::from([("github".to_owned(), 1200)]),
        skills: BTreeMap::from([("review".to_owned(), 40)]),
    });
    let items = claude_units(&source, None);
    let ids: Vec<(ChatToolKind, &str, &str)> = items.iter().map(|i| (i.kind, i.id.as_str(), i.group.as_str())).collect();
    assert_eq!(
        ids,
        [
            (Server, "bridge", "Bridge"),
            (Server, "github", "MCP servers"),
            (Server, "claude.ai Gmail", "claude.ai"),
            (Server, "plugin:chrome-devtools-mcp:chrome-devtools", "chrome-devtools-mcp"),
            (Server, "linear", "MCP servers"),
            (Skill, "review", "Skills"),
            (Skill, "chrome-devtools-mcp:a11y-debugging", "chrome-devtools-mcp"),
            (Plugin, "chrome-devtools-mcp", "Plugins"),
        ]
    );
    assert_eq!(item(&items, Server, "linear").label, "Linear");
    let github = item(&items, Server, "github").tokens.clone().unwrap();
    assert_eq!((github.value, github.source.as_str(), github.at.as_deref()), (1200, "claude.getContextUsage", Some("2026-11-19T10:00:00Z")));
    assert_eq!(item(&items, Skill, "review").tokens.as_ref().map(|t| t.value), Some(40));
    assert_eq!(item(&items, Server, "linear").tokens, None);
    assert!(items.iter().all(|i| i.seen));
    assert!(items.iter().filter(|i| i.kind != Plugin && i.id != "bridge").all(|i| i.state == On));

    // A row of another harness changes nothing.
    let codex = row(json!({"harness": "codex", "serversOff": ["github"]}));
    assert_eq!(claude_units(&source, Some(&codex)), items);
}

#[test]
fn codex_units_are_each_configured_server_codex_apps_and_skills_without_a_table() {
    let tools = json!({"search": {"name": "search", "description": "Search code", "inputSchema": {}}});
    let source = CodexSource {
        status: Some(via("servers", json!({"data": [
            {"name": "bridge", "tools": {"open": {"name": "open", "inputSchema": {}}}},
            {"name": "github", "tools": tools},
            {"name": "empty", "tools": {}},
            {"name": "codex_apps", "tools": {"gmail.send": {}, "gmail.read": {}, "drive.list": {}}}
        ]}))),
        skills: Some(via("skills", json!({"data": [{"cwd": "/w", "errors": [], "skills": [{"name": "review", "description": "Review a diff", "enabled": true}]}]}))),
        config: Some(via("config", json!({"config": {"model": "gpt"}, "origins": {}}))),
        at: Some("2026-11-19T10:00:00Z".into()),
    };
    let items = codex_units(&source, None);
    let ids: Vec<(ChatToolKind, &str)> = items.iter().map(|i| (i.kind, i.id.as_str())).collect();
    assert_eq!(
        ids,
        [(Server, "bridge"), (Server, "github"), (Server, "empty"), (Apps, "codex_apps"), (App, "drive"), (App, "gmail"), (Skill, "review")]
    );
    let github = item(&items, Server, "github");
    assert_eq!(github.state, On);
    let figure = github.tokens.clone().unwrap();
    assert_eq!(figure.value, u64::try_from(tools.to_string().len() / 4).unwrap());
    assert_eq!((figure.source.as_str(), figure.at.as_deref()), (CODEX_SERVER_SOURCE, Some("2026-11-19T10:00:00Z")));
    assert_eq!(item(&items, Server, "empty").tokens, None);
    assert_eq!(item(&items, Apps, "codex_apps").state, On);
    assert!(item(&items, Apps, "codex_apps").tokens.is_some());
    let review = item(&items, Skill, "review");
    assert_eq!(review.state, On);
    assert_eq!(review.tokens.as_ref().map(|t| (t.value, t.source.as_str())), Some((4, CODEX_SKILL_SOURCE)));

    let saved = row(json!({"harness": "codex", "serversOff": ["github"], "codexAppsOff": true, "codexSkillsOff": ["review"]}));
    let items = codex_units(&source, Some(&saved));
    assert_eq!(item(&items, Server, "github").state, Off);
    assert_eq!(item(&items, Apps, "codex_apps").state, Off);
    assert_eq!(item(&items, Skill, "review").state, Off);

    // A skills table in the config fixes each skill.
    let table = CodexSource { config: Some(via("config", json!({"config": {"skills": {"config": []}}}))), ..source.clone() };
    assert_eq!(item(&codex_units(&table, Some(&saved)), Skill, "review").state, Fixed);
    // The own `-c` skills value of Bridge is not a table of the user.
    let layers = json!([
        {"name": {"type": "sessionFlags"}, "version": "v", "config": {"skills": {"config": [{"name": "review", "enabled": false}]}}},
        {"name": {"type": "user", "file": "/h/.codex/config.toml"}, "version": "v", "config": {"model": "gpt"}}
    ]);
    let ours = CodexSource {
        config: Some(via("config", json!({"config": {"skills": {"config": []}}, "origins": {}, "layers": layers}))),
        ..source.clone()
    };
    assert_eq!(item(&codex_units(&ours, Some(&saved)), Skill, "review").state, Off);
}

#[test]
fn opencode_units_are_each_server_and_skills_are_fixed() {
    let v1 = OpenCodeSource { mcp: Some(json!({"bridge": {"status": "connected"}, "github": {"status": "connected"}})) };
    let v2 = OpenCodeSource { mcp: Some(json!({"data": [{"name": "bridge"}, {"name": "github"}]})) };
    for source in [v1, v2] {
        let items = opencode_units(&source, None);
        let ids: Vec<(ChatToolKind, &str, ChatToolState)> = items.iter().map(|i| (i.kind, i.id.as_str(), i.state)).collect();
        assert_eq!(ids, [(Server, "bridge", Fixed), (Server, "github", On), (AllSkills, "skills", Fixed)]);
        assert!(items.iter().all(|i| i.tokens.is_none()));
    }
    let items = opencode_units(&OpenCodeSource::default(), None);
    assert_eq!(items.len(), 1);
    assert_eq!(item(&items, AllSkills, "skills").state, Fixed);
}

#[test]
fn an_item_without_a_verified_lever_is_fixed_with_a_reason() {
    let claude = ClaudeSource { skills: vec!["a(b".into()], plugins: vec!["superpowers".into()], ..ClaudeSource::default() };
    let codex = CodexSource {
        status: Some(via("servers", json!({"data": [{"name": "my.server", "tools": {}}, {"name": "codex_apps", "tools": {"gmail.send": {}}}]}))),
        skills: Some(via("skills", json!({"data": [{"cwd": "/w", "errors": [], "skills": [{"name": "review", "description": ""}]}]}))),
        config: Some(via("config", json!({"config": {"skills": {}, "plugins": {"github@openai-curated": {"enabled": true}}}}))),
        at: None,
    };
    let unknown = CodexSource { config: None, ..codex.clone() };
    let items: Vec<ChatToolItem> = [
        claude_units(&claude, None),
        codex_units(&codex, None),
        codex_units(&unknown, None),
        opencode_units(&OpenCodeSource::default(), None),
    ]
    .concat();
    let fixed: Vec<(ChatToolKind, &str)> = items.iter().filter(|i| i.state == Fixed).map(|i| (i.kind, i.id.as_str())).collect();
    assert_eq!(
        fixed,
        [
            (Skill, "a(b"),
            (Plugin, "superpowers"),
            (Server, "my.server"),
            (App, "gmail"),
            (Skill, "review"),
            (Plugin, "github@openai-curated"),
            (Server, "my.server"),
            (App, "gmail"),
            (Skill, "review"),
            (AllSkills, "skills"),
        ]
    );
    for item in &items {
        assert_eq!(item.state == Fixed, item.fixed_reason.as_deref().is_some_and(|r| !r.is_empty()), "{item:?}");
    }
    let reasons: BTreeSet<&str> = items.iter().filter_map(|i| i.fixed_reason.as_deref()).collect();
    assert!(reasons.contains(CODEX_SKILLS_TABLE_REASON) && reasons.contains(CODEX_SKILLS_UNKNOWN_REASON));
}

#[test]
fn the_bridge_server_is_fixed_on_for_each_harness() {
    // The checks refuse a Bridge entry, so these rows hold one by other means.
    let forged = |harness: &str| ToolSelectionRow { servers_off: vec!["bridge".into()], ..row(json!({"harness": harness})) };
    let items = [
        claude_units(&ClaudeSource { servers: vec!["bridge".into()], ..ClaudeSource::default() }, Some(&forged("claude"))),
        codex_units(&codex_status(json!([{"name": "bridge", "tools": {}}])), Some(&forged("codex"))),
        opencode_units(&OpenCodeSource { mcp: Some(json!({"bridge": {}})) }, Some(&forged("opencode"))),
    ];
    for (items, harness) in items.iter().zip(["claude", "codex", "opencode"]) {
        let bridge = item(items, Server, "bridge");
        assert_eq!((bridge.state, bridge.fixed_reason.as_deref(), bridge.seen), (Fixed, Some(BRIDGE_REASON), true), "{harness}");
        assert_eq!(items.iter().filter(|i| i.id == "bridge").count(), 1, "{harness}");
        assert!(ToolLaunch::from_row(&forged(harness)).servers_off.is_empty(), "{harness}");
    }
}

/// A fixed generator (xorshift64 from one seed): the same choices on each run.
struct Choices(u64);

impl Choices {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn pick(&mut self, pool: &[&str]) -> Vec<String> {
        let mask = self.next();
        let picked: Vec<String> = pool.iter().enumerate().filter(|(at, _)| (mask >> at) & 1 == 1).map(|(_, name)| (*name).to_owned()).collect();
        sorted(picked.iter())
    }

    /// A launch that skipped the checks: it can hold the Bridge server,
    /// unsafe keys, quotes and items of another harness.
    fn launch(&mut self, harness: &str) -> ToolLaunch {
        const SERVERS: [&str; 10] = ["github", "sentry", "linear", "bridge", "Bridge", "codex_apps", "my.server", "a\"b", "x.enabled=true", "approval_policy"];
        const SKILLS: [&str; 8] = ["review", "deploy", "plugin:skill", "say \"hi\"", "back\\slash", "tab\tname", "x\",enabled=true}", "é"];
        let servers_off = self.pick(&SERVERS);
        let codex_apps_off = self.next() & 1 == 1;
        let claude_skills = match self.next() % 3 {
            0 => None,
            1 => Some(Vec::new()),
            _ => Some(self.pick(&SKILLS)),
        };
        let codex_skills_off = self.pick(&SKILLS);
        ToolLaunch { revision: 1, harness: harness.to_owned(), servers_off, codex_apps_off, claude_skills, codex_skills_off, effective_hash: String::new() }
    }
}

/// Claude: the selection holds only a deny list of left-out servers and a
/// skill list. The sidecar adds the deny names to its own list as a union,
/// and a skill list gives `Skill(<name>)` rules in place of `Skill`, so the
/// allow set shrinks and the deny set grows. No other key can carry a mode,
/// a hook, a sandbox, a write scope or an approval.
fn assert_claude_narrows(tools: &ToolLaunch) {
    let Some(selection) = claude_sidecar_selection(tools) else {
        assert_ne!(tools.harness, "claude");
        return;
    };
    let keys: Vec<&str> = selection.as_object().unwrap().keys().map(String::as_str).collect();
    assert!(keys.iter().all(|key| matches!(*key, "deniedMcpServers" | "skills")), "{selection}");
    let denied: Vec<String> = serde_json::from_value(selection["deniedMcpServers"].clone()).unwrap();
    let expected: Vec<&String> = tools.servers_off.iter().filter(|name| !is_bridge(name)).collect();
    assert_eq!(denied.iter().collect::<Vec<_>>(), expected, "{selection}");
    match selection.get("skills") {
        None => assert_eq!(tools.claude_skills, None),
        // Always a list, never the "all" value or a pattern of the baseline.
        Some(skills) => assert_eq!(Some(serde_json::from_value::<Vec<String>>(skills.clone()).unwrap()), tools.claude_skills, "{selection}"),
    }
}

/// Codex: each value has one of the three forms and only turns a thing off.
fn assert_codex_narrows(tools: &ToolLaunch) {
    let values = codex_overrides(tools);
    if tools.harness != "codex" {
        assert!(values.is_empty(), "{values:?}");
        return;
    }
    let mut skills = Vec::new();
    for value in &values {
        let key = value.split_once('=').map_or(value.as_str(), |(key, _)| key).to_ascii_lowercase();
        for prefix in ["approval", "sandbox", "permissions", "mcp_servers.bridge"] {
            assert!(!key.starts_with(prefix), "{value}");
        }
        match parse_codex_off(value) {
            Some(CodexOff::Server(key)) => {
                assert!(value.ends_with(".enabled=false") && tools.servers_off.contains(&key), "{value}");
                assert!(!is_bridge(&key) && key != CODEX_APPS, "{value}");
            }
            Some(CodexOff::Apps) => assert!(tools.codex_apps_off && value == CODEX_APPS_OFF),
            Some(CodexOff::Skills(names)) => skills.push(names),
            None => panic!("{value} has none of the three forms"),
        }
    }
    assert_eq!(values.iter().filter(|value| *value == CODEX_APPS_OFF).count(), usize::from(tools.codex_apps_off));
    let expected: Vec<Vec<String>> = (!tools.codex_skills_off.is_empty()).then(|| tools.codex_skills_off.clone()).into_iter().collect();
    assert_eq!(skills, expected);
}

/// OpenCode: the content equals the input plus `enabled: false` leaves, and
/// the permission object and the Bridge entry equal the baseline.
fn assert_opencode_narrows(tools: &ToolLaunch, baseline: &Value) {
    let OpenCodeMerge::Applied(content) = merge_opencode(Some(&baseline.to_string()), &tools.servers_off) else {
        panic!("a JSON object always merges");
    };
    let mut content: Value = serde_json::from_str(&content).unwrap();
    assert_eq!(content["permission"], baseline["permission"]);
    assert_eq!(content["mcp"]["bridge"], baseline["mcp"]["bridge"]);
    let servers = content["mcp"].as_object_mut().unwrap();
    for name in tools.servers_off.iter().filter(|name| !is_bridge(name)) {
        let entry = servers.get_mut(name.as_str()).and_then(Value::as_object_mut).unwrap();
        assert_eq!(entry.remove("enabled"), Some(Value::Bool(false)), "{name}");
        match baseline["mcp"].get(name.as_str()) {
            Some(before) => {
                if let Some(enabled) = before.get("enabled") {
                    entry.insert("enabled".into(), enabled.clone());
                }
            }
            None => {
                assert!(entry.is_empty(), "{name}");
                servers.remove(name.as_str());
            }
        }
    }
    assert_eq!(&content, baseline);
}

#[test]
fn no_selection_widens_a_permission_on_any_harness() {
    let baseline = json!({
        "theme": "system",
        "permission": {"edit": "ask", "bash": {"*": "ask"}, "github_*": "deny"},
        "mcp": {
            "bridge": {"type": "local", "command": ["/Applications/Bridge.app/Contents/MacOS/bridged", "--bridge-mcp-visualize"], "enabled": true},
            "github": {"type": "local", "command": ["gh-mcp"]},
            "sentry": {"type": "remote", "url": "https://sentry.example", "enabled": true}
        }
    });
    let mut choices = Choices(0x9E37_79B9_7F4A_7C15);
    let mut seen = BTreeSet::new();
    let mut shapes = BTreeSet::new();
    for round in 0..600 {
        let tools = choices.launch(["claude", "codex", "opencode"][round % 3]);
        seen.insert((tools.harness.clone(), tools.servers_off.clone(), tools.codex_skills_off.clone()));
        shapes.insert((tools.servers_off.iter().any(|name| is_bridge(name)), tools.codex_apps_off, tools.claude_skills.as_ref().map(Vec::is_empty)));
        assert_claude_narrows(&tools);
        assert_codex_narrows(&tools);
        assert_opencode_narrows(&tools, &baseline);
    }
    // The generator gives many distinct choices and each shape: with and
    // without the Bridge server, apps on and off, and each skill choice.
    assert!(seen.len() > 500, "{}", seen.len());
    assert_eq!(shapes.len(), 12, "{shapes:?}");
}

// --- save for a chat that exists --------------------------------------------------

fn core() -> (tempfile::TempDir, BridgeCore) {
    let dir = tempfile::tempdir().unwrap();
    let core = BridgeCore::for_tests(dir.path());
    (dir, core)
}

fn github() -> ChatToolSelection {
    choice(json!({"harness": "claude", "serversOff": ["github"]}))
}

fn set(core: &BridgeCore, sql: &str) {
    core.db.lock().unwrap().execute(sql, []).unwrap();
}

fn stored(core: &BridgeCore, id: &str) -> Option<ToolSelectionRow> {
    read(&core.db.lock().unwrap(), id).unwrap()
}

fn busy(core: &BridgeCore, id: &str) -> SaveOutcome {
    save(core, id, 0, &github()).unwrap()
}

/// The first save of the chat goes through and stores revision 1.
fn assert_saved(core: &BridgeCore, id: &str) {
    let outcome = save(core, id, 0, &github()).unwrap();
    let SaveOutcome::Saved(row) = outcome else { panic!("expected a save, got {outcome:?}") };
    assert_eq!((row.revision, row.servers_off.clone()), (1, vec!["github".to_owned()]));
    assert_eq!(stored(core, id), Some(row));
}

#[derive(Default)]
struct Calls {
    send: AtomicUsize,
    interrupt: AtomicUsize,
    stop: AtomicUsize,
}

/// A warm runtime that counts each call that could disturb the process.
#[derive(Default)]
struct Warm {
    turn: Arc<Mutex<Option<String>>>,
    calls: Arc<Calls>,
}

impl crate::adapters::AdapterRuntime for Warm {
    fn process_id(&self) -> u32 {
        4242
    }
    fn provider_session_id(&self) -> &str {
        "provider-1"
    }
    fn current_turn(&self) -> Arc<Mutex<Option<String>>> {
        Arc::clone(&self.turn)
    }
    fn send_turn(&self, _: &str) -> Result<(), BridgeError> {
        self.calls.send.fetch_add(1, SeqCst);
        Ok(())
    }
    fn interrupt(&self) -> Result<(), BridgeError> {
        self.calls.interrupt.fetch_add(1, SeqCst);
        Ok(())
    }
    fn respond(&self, _: Value, _: &str) -> Result<(), BridgeError> {
        Ok(())
    }
    fn stop(&mut self, _: crate::adapters::ShutdownReason) {
        self.calls.stop.fetch_add(1, SeqCst);
    }
}

#[test]
fn a_save_is_refused_while_the_chat_is_working_or_checkpointing() {
    let (_dir, core) = core();
    chat(&core.db.lock().unwrap(), "c", "claude", None);
    for (status, reason) in [
        ("working", TURN_BUSY),
        ("checkpointing", CHECKPOINT_BUSY),
        ("waiting", APPROVAL_BUSY),
        ("starting", START_BUSY),
        ("completed", STATUS_BUSY),
    ] {
        core.db.lock().unwrap().execute("UPDATE sessions SET status=?1 WHERE id='c'", [status]).unwrap();
        assert_eq!(busy(&core, "c"), SaveOutcome::Busy(reason), "{status}");
    }

    // A ready row with an active turn still runs that turn.
    set(&core, "UPDATE sessions SET status='ready',active_turn_id='turn-1' WHERE id='c'");
    assert_eq!(busy(&core, "c"), SaveOutcome::Busy(TURN_BUSY));

    // A warm runtime can run a turn that the row does not show yet.
    set(&core, "UPDATE sessions SET active_turn_id=NULL WHERE id='c'");
    let warm = Warm::default();
    *warm.turn.lock().unwrap() = Some("turn-2".to_owned());
    let turn = Arc::clone(&warm.turn);
    core.adapters.lock().unwrap().insert("c".to_owned(), Box::new(warm));
    assert_eq!(busy(&core, "c"), SaveOutcome::Busy(TURN_BUSY));

    // Another change to the chat holds its lifecycle claim.
    *turn.lock().unwrap() = None;
    let claim = core.claim_session_lifecycle("c", "session start").unwrap();
    assert_eq!(busy(&core, "c"), SaveOutcome::Busy(LIFECYCLE_BUSY));
    drop(claim);

    // No refusal stored a row or kept the claim, so the idle chat saves.
    assert_eq!(stored(&core, "c"), None);
    assert!(core.input_activity.try_write().is_ok());
    assert_saved(&core, "c");
}

#[test]
fn a_save_is_refused_with_queued_input_a_waiting_subagent_or_a_pending_worker_result() {
    let (_dir, core) = core();
    chat(&core.db.lock().unwrap(), "c", "claude", None);

    crate::session_input::enqueue(&core.db.lock().unwrap(), "c", "next", "next").unwrap();
    assert_eq!(busy(&core, "c"), SaveOutcome::Busy(QUEUED_BUSY));
    set(&core, "DELETE FROM queued_session_input");

    // A native subagent waits for an answer, however long ago it started.
    set(&core, "INSERT INTO sessions(id,harness,label,status,metric_source,kind,parent_session_id,started_at) VALUES('sub','claude','Subagent','waiting','reported','harness_subagent','c','2020-01-01T00:00:00+00:00')");
    assert_eq!(busy(&core, "c"), SaveOutcome::Busy(SUBAGENT_BUSY));
    set(&core, "UPDATE sessions SET status='completed' WHERE id='sub'");

    // A worker finished, but its result did not reach the chat.
    set(&core, "INSERT INTO sessions(id,harness,label,status,metric_source,kind,parent_session_id) VALUES('worker','claude','Worker','completed','reported','direct','c')");
    set(&core, "INSERT INTO worker_runtime(session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,result_status,updated_at) VALUES('worker','c','completed','research','key','pending','now')");
    assert_eq!(busy(&core, "c"), SaveOutcome::Busy(WORKER_BUSY));
    set(&core, "UPDATE worker_runtime SET result_status='reported' WHERE session_id='worker'");

    // The same result can wait in the outbox.
    let payload = json!({"report": {"parent_session_id": "c"}}).to_string();
    core.db
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO durable_outbox(id,destination,event_type,payload,idempotency_key,status,next_attempt_at,created_at) VALUES('result','parent','worker.result',?1,'result','pending','now','now')",
            [payload],
        )
        .unwrap();
    assert_eq!(busy(&core, "c"), SaveOutcome::Busy(WORKER_BUSY));
    set(&core, "UPDATE durable_outbox SET status='delivered'");

    set(&core, "INSERT INTO session_background_tasks(session_id,item_id,sequence,in_progress) VALUES('c','task',1,1)");
    assert_eq!(busy(&core, "c"), SaveOutcome::Busy(TASK_BUSY));
    set(&core, "UPDATE session_background_tasks SET in_progress=0");

    assert_eq!(stored(&core, "c"), None);
    assert_saved(&core, "c");
}

#[test]
fn a_save_is_refused_for_a_worker_or_a_child_session() {
    let (_dir, core) = core();
    {
        let db = core.db.lock().unwrap();
        chat(&db, "c", "claude", None);
        for (id, kind, parent) in [
            ("child", "direct", Some("c")),
            ("subagent", "harness_subagent", Some("c")),
            ("flow", "workflow", None),
        ] {
            db.execute(
                "INSERT INTO sessions(id,harness,label,status,metric_source,kind,parent_session_id) VALUES(?1,'claude','Other','ready','reported',?2,?3)",
                params![id, kind, parent],
            )
            .unwrap();
        }
        // A worker row has no parent column, but its runtime row names it.
        chat(&db, "worker", "claude", None);
        db.execute("INSERT INTO worker_runtime(session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,result_status,updated_at) VALUES('worker','c','ready','research','key','reported','now')", []).unwrap();
    }
    for id in ["child", "subagent", "flow", "worker"] {
        assert_eq!(message(save(&core, id, 0, &github())), "Only a top-level chat can have a tool choice", "{id}");
        assert_eq!(stored(&core, id), None, "{id}");
        assert!(core.claim_session_lifecycle(id, "session start").is_ok(), "{id}");
    }
    assert_eq!(message(save(&core, "missing", 0, &github())), "This chat does not exist");
    assert!(core.input_activity.try_write().is_ok());
    assert_saved(&core, "c");
}

#[test]
fn a_save_is_accepted_after_a_failed_launch() {
    let (_dir, core) = core();
    {
        let db = core.db.lock().unwrap();
        chat(&db, "c", "claude", None);
        write_checked(&db, "c", 0, &github()).unwrap();
        // The launch with that choice failed. These are the columns that a
        // recoverable adapter failure writes.
        db.execute(
            "UPDATE sessions SET status='failed',active_turn_id=NULL,ended_at='2026-11-19T00:00:00+00:00',provider_session_id='provider-1' WHERE id='c'",
            [],
        )
        .unwrap();
        // The idle release keeps its list, so it does not take a failed chat.
        assert!(!crate::runtime_budget::idle_for_change(&db, "c", &["ready", "idle", "stopped"]));
        assert!(crate::runtime_budget::idle_for_change(&db, "c", SAVE_STATUSES));
    }

    // The person turns the server on again, and the save goes through.
    let outcome = save(&core, "c", 1, &choice(json!({"harness": "claude"}))).unwrap();
    let SaveOutcome::Saved(row) = outcome else { panic!("expected a save, got {outcome:?}") };
    assert_eq!((row.revision, row.servers_off.clone()), (2, Vec::<String>::new()));
    assert_eq!(stored(&core, "c"), Some(row));
    let session: (String, Option<String>) = core
        .db
        .lock()
        .unwrap()
        .query_row("SELECT status,provider_session_id FROM sessions WHERE id='c'", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(session, ("failed".to_owned(), Some("provider-1".to_owned())));

    // A save on the old revision still fails.
    assert!(message(save(&core, "c", 1, &github())).contains("changed in another window"));
}

#[test]
fn a_save_does_not_stop_the_process() {
    let (_dir, core) = core();
    {
        let db = core.db.lock().unwrap();
        chat(&db, "c", "claude", None);
        db.execute("UPDATE sessions SET status='idle',adapter_pid=4242,provider_session_id='provider-1' WHERE id='c'", []).unwrap();
    }
    let warm = Warm::default();
    let calls = Arc::clone(&warm.calls);
    core.adapters.lock().unwrap().insert("c".to_owned(), Box::new(warm));
    let count = |table: &str| -> i64 {
        core.db
            .lock()
            .unwrap()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))
            .unwrap()
    };
    let before = (count("events"), count("session_entries"));

    assert_saved(&core, "c");
    let outcome = save(&core, "c", 1, &choice(json!({"harness": "claude", "serversOff": ["github", "linear"]}))).unwrap();
    assert!(matches!(&outcome, SaveOutcome::Saved(row) if row.revision == 2), "{outcome:?}");

    // No call reached the process, and the runtime stays warm.
    let counts = (calls.send.load(SeqCst), calls.interrupt.load(SeqCst), calls.stop.load(SeqCst));
    assert_eq!(counts, (0, 0, 0));
    {
        let adapters = core.adapters.lock().unwrap();
        let runtime = adapters.get("c").expect("the warm runtime stays");
        assert_eq!((runtime.process_id(), runtime.provider_session_id()), (4242, "provider-1"));
    }
    let session: (String, Option<i64>, Option<String>) = core
        .db
        .lock()
        .unwrap()
        .query_row("SELECT status,adapter_pid,provider_session_id FROM sessions WHERE id='c'", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap();
    assert_eq!(session, ("idle".to_owned(), Some(4242), Some("provider-1".to_owned())));

    // The save wrote no event and no entry, and it left the guard free.
    assert_eq!((count("events"), count("session_entries")), before);
    assert!(core.input_activity.try_write().is_ok());
    assert!(core.claim_session_lifecycle("c", "session start").is_ok());
}

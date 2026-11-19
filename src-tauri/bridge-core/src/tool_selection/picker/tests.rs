use super::*;
use crate::tool_selection::{write_checked, OPENCODE_CONFIG_REASON, SEND_BUSY};
use bridge_protocol::messages::ChatToolKind::{AllSkills, App, Apps, Server, Skill};
use bridge_protocol::messages::ChatToolState::{Fixed, Off, On};
use bridge_protocol::messages::{ChatToolKind, ChatToolSourceKind as Kind};
use std::sync::Mutex;

fn core() -> (tempfile::TempDir, Arc<BridgeCore>) {
    let dir = tempfile::tempdir().unwrap();
    let core = Arc::new(BridgeCore::for_tests(dir.path()));
    {
        let db = core.db.lock().unwrap();
        db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','P','/tmp','now')", []).unwrap();
        for id in ["a", "b"] {
            db.execute(
                "INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES(?1,'p','Oslo',?1,'main',?2,'ready','now')",
                [id, &format!("/tmp/{id}")],
            )
            .unwrap();
        }
    }
    (dir, core)
}

fn chat(core: &BridgeCore, id: &str, harness: &str, account: Option<&str>, workspace: Option<&str>) {
    core.db
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,title,status,metric_source,kind,account) VALUES(?1,?2,?3,'Chat',?4,'idle','reported','direct',?5)",
            params![id, workspace, harness, format!("Chat {id}"), account],
        )
        .unwrap();
}

fn init(core: &BridgeCore, id: &str, sequence: i64, servers: &[&str], skills: &[&str]) {
    let data = json!({
        "subtype": "init",
        "session_id": format!("provider-{id}"),
        "mcp_servers": servers.iter().map(|name| json!({"name": name, "status": "connected"})).collect::<Vec<_>>(),
        "skills": skills,
        "plugins": [{"name": "superpowers", "path": "/plugins/superpowers"}],
    });
    core.db
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO session_entries(id,session_id,sequence,kind,payload,created_at) VALUES(?1,?2,?3,'session.started',?4,'2026-11-19T10:00:00Z')",
            params![format!("{id}-{sequence}"), id, sequence, json!({"data": data}).to_string()],
        )
        .unwrap();
}

fn reading(core: &BridgeCore, id: &str, breakdown: &Value) {
    core.db
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO context_readings(session_id,harness,used_tokens,window_tokens,state,source,breakdown_json,created_at) VALUES(?1,'claude',1000,200000,'measured','claude.context_usage',?2,'2026-11-19T10:05:00Z')",
            params![id, breakdown.to_string()],
        )
        .unwrap();
}

fn save_row(core: &BridgeCore, id: &str, choice: Value) {
    write_checked(&core.db.lock().unwrap(), id, 0, &serde_json::from_value(choice).unwrap()).unwrap();
}

fn of_session(core: &Arc<BridgeCore>, id: &str) -> GetChatToolsResult {
    get(core, &GetChatToolsParams { session_id: Some(id.to_owned()), draft: None }).unwrap()
}

fn of_draft(core: &Arc<BridgeCore>, harness: &str, account: Option<&str>, workspace: Option<&str>) -> GetChatToolsResult {
    let draft = ChatToolsDraft {
        workspace_id: workspace.map(str::to_owned),
        harness: harness.to_owned(),
        account: account.map(str::to_owned),
    };
    get(core, &GetChatToolsParams { session_id: None, draft: Some(draft) }).unwrap()
}

fn item<'a>(result: &'a GetChatToolsResult, kind: ChatToolKind, id: &str) -> &'a ChatToolItem {
    result
        .items
        .iter()
        .find(|item| item.kind == kind && item.id == id)
        .unwrap_or_else(|| panic!("no {kind:?} {id} in {:#?}", result.items))
}

fn source_of(result: &GetChatToolsResult, kind: Kind) -> &ChatToolSource {
    result.sources.iter().find(|source| source.kind == kind).unwrap_or_else(|| panic!("no {kind:?} in {:#?}", result.sources))
}

/// A warm Codex runtime. It answers each read through the app-control path,
/// or fails each one, and counts each call that could start work.
struct Codex {
    owner: &'static str,
    asked: Arc<Mutex<Vec<String>>>,
    fail: bool,
}

impl crate::adapters::AdapterRuntime for Codex {
    fn process_id(&self) -> u32 {
        4242
    }
    fn provider_session_id(&self) -> &str {
        "thread"
    }
    fn current_turn(&self) -> Arc<Mutex<Option<String>>> {
        Arc::new(Mutex::new(None))
    }
    fn send_turn(&self, _: &str) -> Result<(), BridgeError> {
        panic!("the tools list never starts a turn")
    }
    fn interrupt(&self) -> Result<(), BridgeError> {
        Ok(())
    }
    fn respond(&self, _: Value, _: &str) -> Result<(), BridgeError> {
        Ok(())
    }
    fn stop(&mut self, _: crate::adapters::ShutdownReason) {}
    fn app_control(&self, id: &str, request: &Value) -> Result<(), BridgeError> {
        let kind = request["kind"].as_str().unwrap_or_default().to_owned();
        self.asked.lock().unwrap().push(kind.clone());
        let result = match kind.as_str() {
            "servers" => json!({"servers": [
                {"name": "bridge", "status": "connected", "connectors": [], "tokens": 300},
                {"name": "github", "status": "connected", "connectors": [], "tokens": 1200},
                {"name": "codex_apps", "status": "connected", "connectors": ["Gmail", "Notion"], "tokens": 900}
            ]}),
            "skills" => json!({"skills": [{"name": "lint", "enabled": true, "tokens": 25}]}),
            _ => json!({"userSkillsKey": false, "plugins": []}),
        };
        let frame = json!({"type": "app_control_result", "id": id, "ok": !self.fail, "result": result});
        assert!(crate::apps::control::deliver(self.owner, &frame));
        Ok(())
    }
}

/// A warm OpenCode runtime that answers the MCP list with a V1 map.
struct OpenCode {
    listed: Value,
}

impl crate::adapters::AdapterRuntime for OpenCode {
    fn process_id(&self) -> u32 {
        4343
    }
    fn provider_session_id(&self) -> &str {
        "ses_1"
    }
    fn current_turn(&self) -> Arc<Mutex<Option<String>>> {
        Arc::new(Mutex::new(None))
    }
    fn send_turn(&self, _: &str) -> Result<(), BridgeError> {
        panic!("the tools list never starts a turn")
    }
    fn interrupt(&self) -> Result<(), BridgeError> {
        Ok(())
    }
    fn respond(&self, _: Value, _: &str) -> Result<(), BridgeError> {
        Ok(())
    }
    fn stop(&mut self, _: crate::adapters::ShutdownReason) {}
    fn mcp_list_request(&self) -> Option<crate::adapters::McpListRequest> {
        let listed = self.listed.clone();
        Some(Box::new(move || Ok(listed)))
    }
}

#[test]
fn a_cold_codex_chat_keeps_its_choice_and_waits_for_the_first_turn() {
    let (_dir, core) = core();
    chat(&core, "codex-cold", "codex", None, Some("a"));
    save_row(&core, "codex-cold", json!({"harness": "codex", "serversOff": ["github"], "codexAppsOff": true}));

    let result = of_session(&core, "codex-cold");
    assert_eq!((result.harness.as_str(), result.selection.revision, result.selection.applies), ("codex", 1, true));
    assert_eq!(result.choice.as_ref().unwrap().servers_off, vec!["github".to_owned()]);
    let github = item(&result, Server, "github");
    assert_eq!((github.state, github.seen), (Off, false), "a saved name stays visible");
    assert_eq!(item(&result, Apps, "codex_apps").state, Off);
    let runtime = source_of(&result, Kind::Runtime);
    assert!(!runtime.available);
    assert_eq!(runtime.note.as_deref(), Some(COLD_NOTE));
    // No runtime started for the read.
    assert!(core.adapters.lock().unwrap().is_empty());

    let draft = of_draft(&core, "codex", None, Some("a"));
    assert_eq!(draft.selection.revision, 0);
    assert_eq!(draft.choice, None);
    assert_eq!(draft.items.iter().map(|item| (item.kind, item.id.as_str())).collect::<Vec<_>>(), [(Apps, "codex_apps")]);
    assert_eq!(source_of(&draft, Kind::Runtime).note.as_deref(), Some(COLD_NOTE));
    assert!(core.adapters.lock().unwrap().is_empty());
}

#[test]
fn a_warm_codex_chat_reads_servers_skills_and_config_from_its_runtime() {
    let (_dir, core) = core();
    chat(&core, "codex-chat", "codex", None, Some("a"));
    let asked = Arc::new(Mutex::new(Vec::new()));
    core.adapters
        .lock()
        .unwrap()
        .insert("codex-chat".to_owned(), Box::new(Codex { owner: "codex-chat", asked: Arc::clone(&asked), fail: false }));

    let result = of_session(&core, "codex-chat");
    let mut kinds = asked.lock().unwrap().clone();
    kinds.sort();
    assert_eq!(kinds, ["config", "servers", "skills"], "three reads and no write");
    let github = item(&result, Server, "github");
    assert_eq!((github.state, github.seen), (On, true));
    let tokens = github.tokens.as_ref().unwrap();
    assert_eq!((tokens.value, tokens.source.as_str()), (1200, crate::tool_selection::CODEX_SERVER_SOURCE));
    assert!(tokens.at.is_some(), "each figure names its time");
    assert_eq!(item(&result, Server, "bridge").state, Fixed);
    assert_eq!(item(&result, Apps, "codex_apps").state, On);
    assert_eq!(item(&result, App, "Gmail").state, Fixed);
    let lint = item(&result, Skill, "lint");
    assert_eq!((lint.state, lint.tokens.as_ref().unwrap().source.as_str()), (On, crate::tool_selection::CODEX_SKILL_SOURCE));
    assert_eq!(result.sources.iter().filter(|source| source.kind == Kind::Runtime && source.available).count(), 3);
    let place = CodexPlace::resolve(&core.db.lock().unwrap(), None, "/tmp/a").unwrap();
    let kept = codex_skills_key(&core.db.lock().unwrap(), "codex-chat", Some(&place));
    assert_eq!(kept, CodexSkillsKey::Known(false), "the config read is kept");
}

#[test]
fn a_warm_read_that_confirms_the_config_ends_a_partial_launch() {
    let (_dir, core) = core();
    let applied = |partial: bool| crate::tool_selection::AppliedTools {
        revision: 1,
        effective_hash: "hash".into(),
        confirmed: true,
        pending: None,
        partial,
        started_at: "launch".into(),
    };
    for (id, partial) in [("codex-partial", true), ("codex-whole", false)] {
        chat(&core, id, "codex", None, Some("a"));
        save_row(&core, id, json!({"harness": "codex", "codexSkillsOff": ["lint"]}));
        crate::tool_selection::record_applied(&core, id, applied(partial));
        let owner = if partial { "codex-partial" } else { "codex-whole" };
        core.adapters.lock().unwrap().insert(id.to_owned(), Box::new(Codex { owner, asked: Arc::default(), fail: false }));
        let result = of_session(&core, id);
        assert_eq!(item(&result, Skill, "lint").state, Off, "{id}");
    }
    // The runtime that dropped the skills part relaunches at the next send.
    assert_eq!(crate::tool_selection::applied_revision(&core, "codex-partial"), 0);
    assert!(crate::tool_selection::stale(&core, "codex-partial"));
    assert_eq!(crate::tool_selection::applied_revision(&core, "codex-whole"), 1);
}

#[test]
fn a_failed_read_never_drops_a_saved_choice() {
    let (_dir, core) = core();
    chat(&core, "codex-fail", "codex", None, None);
    save_row(&core, "codex-fail", json!({"harness": "codex", "serversOff": ["github"], "codexSkillsOff": ["lint"]}));
    let asked = Arc::new(Mutex::new(Vec::new()));
    core.adapters.lock().unwrap().insert("codex-fail".to_owned(), Box::new(Codex { owner: "codex-fail", asked, fail: true }));

    let result = of_session(&core, "codex-fail");
    assert_eq!(result.choice.as_ref().unwrap().codex_skills_off, vec!["lint".to_owned()]);
    let github = item(&result, Server, "github");
    assert_eq!((github.state, github.seen), (Off, false));
    let lint = item(&result, Skill, "lint");
    assert!(!lint.seen);
    let failed: Vec<&ChatToolSource> = result.sources.iter().filter(|source| source.kind == Kind::Runtime).collect();
    assert_eq!(failed.len(), 3);
    assert!(failed.iter().all(|source| !source.available && source.note.is_some()), "{failed:#?}");

    // A row that fails its checks keeps its revision, so a save can replace it.
    let db = core.db.lock().unwrap();
    crate::chat_defaults::write(&db, crate::tool_selection::KIND, "codex-fail", &json!({"revision": 4, "harness": "codex", "serversOff": ["bridge"]})).unwrap();
    drop(db);
    core.adapters.lock().unwrap().clear();
    let result = of_session(&core, "codex-fail");
    assert_eq!((result.selection.revision, result.selection.applies), (4, false));
    assert!(result.selection.not_applied_reason.unwrap().contains("not valid"));
    assert_eq!(result.choice, None);
}

#[test]
fn an_opencode_chat_reads_the_mcp_list_of_its_warm_server() {
    let (_dir, core) = core();
    chat(&core, "oc", "opencode", None, None);
    save_row(&core, "oc", json!({"harness": "opencode", "serversOff": ["gone"]}));
    let listed = json!({"fake": {"status": "connected"}, "bridge": {"status": "connected"}});
    core.adapters.lock().unwrap().insert("oc".to_owned(), Box::new(OpenCode { listed }));

    let result = of_session(&core, "oc");
    assert_eq!(item(&result, Server, "fake").state, On);
    assert!(item(&result, Server, "fake").tokens.is_none(), "OpenCode reports no figure");
    assert_eq!(item(&result, Server, "bridge").state, Fixed);
    assert_eq!((item(&result, Server, "gone").state, item(&result, Server, "gone").seen), (Off, false));
    assert_eq!(item(&result, AllSkills, "skills").state, Fixed);
    assert!(source_of(&result, Kind::Runtime).available);

    let draft = of_draft(&core, "opencode", None, None);
    assert_eq!(source_of(&draft, Kind::Runtime).note.as_deref(), Some(COLD_NOTE));
}

#[test]
fn an_inherited_opencode_config_that_does_not_parse_shows_as_not_applied() {
    let selection = selection_status(
        &Chat { harness: "opencode".into(), account: None, workspace_id: None },
        2,
        None,
    );
    let choice = ChatToolSelection {
        harness: "opencode".into(),
        account: None,
        servers_off: vec!["fake".into()],
        codex_apps_off: false,
        claude_skills: None,
        codex_skills_off: Vec::new(),
    };
    // The guard reads the environment of Bridge, so the test calls the same
    // check with a fixed value.
    assert_eq!(opencode_not_applied_reason(Some("{bad"), &choice.servers_off), Some(OPENCODE_CONFIG_REASON));
    let guarded = not_applied(selection.clone(), OPENCODE_CONFIG_REASON);
    assert_eq!((guarded.applies, guarded.not_applied_reason.as_deref()), (false, Some(OPENCODE_CONFIG_REASON)));
    assert_eq!(guarded.revision, 2);
    let kept = not_applied(ChatToolSelectionStatus { applies: false, not_applied_reason: Some("Chosen for Codex".into()), ..selection }, OPENCODE_CONFIG_REASON);
    assert_eq!(kept.not_applied_reason.as_deref(), Some("Chosen for Codex"), "the first reason stays");
}

#[test]
fn a_claude_chat_lists_its_last_init_with_the_figures_of_its_reading() {
    let (_dir, core) = core();
    chat(&core, "cl", "claude", None, Some("a"));
    init(&core, "cl", 1, &["bridge", "github"], &["old"]);
    init(&core, "cl", 2, &["bridge", "github", "claude.ai Gmail"], &["review", "superpowers:brainstorming"]);
    reading(&core, "cl", &json!({"mcpServers": [{"name": "github", "tokens": 1500}], "skills": {"items": [{"name": "review", "tokens": 40}]}}));
    save_row(&core, "cl", json!({"harness": "claude", "serversOff": ["claude.ai Gmail"], "claudeSkills": {"only": ["review"]}}));

    let result = of_session(&core, "cl");
    assert_eq!(item(&result, Server, "claude.ai Gmail").state, Off);
    assert_eq!(item(&result, Skill, "review").state, On);
    assert_eq!(item(&result, Skill, "superpowers:brainstorming").state, Off);
    assert!(result.items.iter().all(|item| item.id != "old"), "only the last init counts");
    let tokens = item(&result, Server, "github").tokens.clone().unwrap();
    assert_eq!((tokens.value, tokens.source.as_str(), tokens.at.as_deref()), (1500, CLAUDE_FIGURE_SOURCE, Some("2026-11-19T10:05:00Z")));
    assert_eq!(item(&result, Skill, "review").tokens.as_ref().unwrap().value, 40);
    assert!(item(&result, Skill, "superpowers:brainstorming").tokens.is_none(), "an item without a figure shows none");
    let init = source_of(&result, Kind::ClaudeInit);
    assert_eq!((init.label.as_str(), init.at.as_deref(), init.note.as_deref()), ("Claude init of this chat", Some("2026-11-19T10:00:00Z"), None));
    assert!(source_of(&result, Kind::ContextReading).available);
    assert_eq!(source_of(&result, Kind::Catalog).kind, Kind::Catalog);
}

#[test]
fn a_claude_draft_borrows_the_newest_init_of_the_same_account_and_workspace_first() {
    let (_dir, core) = core();
    chat(&core, "own-b", "claude", None, Some("b"));
    init(&core, "own-b", 1, &["github"], &[]);
    chat(&core, "own-a", "claude", None, Some("a"));
    init(&core, "own-a", 1, &["linear"], &[]);
    chat(&core, "own-b-newer", "claude", None, Some("b"));
    init(&core, "own-b-newer", 1, &["figma"], &[]);
    chat(&core, "work", "claude", Some("work"), Some("a"));
    init(&core, "work", 1, &["salesforce"], &[]);
    chat(&core, "codex", "codex", None, Some("a"));
    // A child chat never lends its init.
    core.db
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind,parent_session_id) VALUES('child','a','claude','Child','idle','reported','direct','own-a')",
            [],
        )
        .unwrap();
    init(&core, "child", 1, &["secret"], &[]);

    let same_workspace = of_draft(&core, "claude", None, Some("a"));
    assert!(same_workspace.items.iter().any(|item| item.id == "linear"), "{:#?}", same_workspace.items);
    let init = source_of(&same_workspace, Kind::ClaudeInit);
    assert_eq!(init.label, "Claude init of Chat own-a");
    assert_eq!(init.note.as_deref(), Some(DONOR_NOTE));

    let other_workspace = of_draft(&core, "claude", None, None);
    assert!(other_workspace.items.iter().any(|item| item.id == "figma"), "the newest chat wins: {:#?}", other_workspace.items);

    let work = of_draft(&core, "claude", Some("work"), Some("b"));
    assert!(work.items.iter().any(|item| item.id == "salesforce"));
    assert!(work.items.iter().all(|item| item.id != "secret"));
    assert_eq!(work.account.as_deref(), Some("work"));

    let none = of_draft(&core, "claude", Some("nobody"), None);
    let init = source_of(&none, Kind::ClaudeInit);
    assert_eq!((init.available, init.note.as_deref()), (false, Some(COLD_NOTE)));
}

#[test]
fn the_list_refuses_a_child_a_missing_chat_or_two_targets() {
    let (_dir, core) = core();
    chat(&core, "c", "claude", None, None);
    core.db
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO sessions(id,harness,label,status,metric_source,kind,parent_session_id) VALUES('child','claude','Child','idle','reported','direct','c')",
            [],
        )
        .unwrap();
    let refusal = |params: GetChatToolsParams| match get(&core, &params) {
        Err(BridgeError::Invalid(message)) => message,
        other => panic!("expected a refusal, got {other:?}"),
    };
    let session = |id: &str| GetChatToolsParams { session_id: Some(id.to_owned()), draft: None };
    assert_eq!(refusal(session("child")), "Only a top-level chat can have a tool choice");
    assert_eq!(refusal(session("missing")), "This chat does not exist");
    let draft = ChatToolsDraft { workspace_id: None, harness: "claude".into(), account: None };
    assert_eq!(refusal(GetChatToolsParams { session_id: Some("c".into()), draft: Some(draft) }), "Give exactly one of sessionId and draft");
    assert_eq!(refusal(GetChatToolsParams { session_id: None, draft: None }), "Give exactly one of sessionId and draft");
}

#[test]
fn update_returns_the_new_revision_or_the_refusal() {
    let (_dir, core) = core();
    chat(&core, "c", "claude", None, None);
    let selection: ChatToolSelection = serde_json::from_value(json!({"harness": "claude", "serversOff": ["github"]})).unwrap();
    let params = UpdateChatToolsParams { session_id: "c".into(), expected_revision: 0, selection };
    assert_eq!(update(&core, &params).unwrap(), UpdateChatToolsResult { revision: Some(1), refusal: None });

    // A send holds the input lease, so the save is refused and nothing changes.
    let lease = core.input_activity.read().unwrap();
    let refused = update(&core, &UpdateChatToolsParams { expected_revision: 1, ..params.clone() }).unwrap();
    assert_eq!(refused, UpdateChatToolsResult { revision: None, refusal: Some(SEND_BUSY.to_owned()) });
    drop(lease);
    assert_eq!(of_session(&core, "c").selection.revision, 1);

    // A stale revision is an error, not a refusal.
    assert!(matches!(update(&core, &params), Err(BridgeError::Invalid(message)) if message.contains("changed in another window")));
}

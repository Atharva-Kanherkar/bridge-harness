use super::*;
use crate::adapters::{AdapterRegistry, HarnessAdapter, ResumeRequest, StartRequest, StartedAdapter};
use crate::tool_selection::{self, ToolLaunch};
use bridge_protocol::messages::ChatToolSelection;
use serde_json::{json, Value};

type Launch = (&'static str, bool, Option<ToolLaunch>);
type Calls = Arc<Mutex<Vec<Launch>>>;

/// A Claude adapter that records the tool launch of each launch and starts
/// nothing.
struct Recorder(Calls);

impl HarnessAdapter for Recorder {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn descriptor(&self) -> AdapterDescriptor {
        serde_json::from_value(json!({
            "id": "claude", "label": "Claude", "available": true, "authState": "unknown",
            "capabilities": ["messages", "briefings"],
            "sandboxModes": ["read_only", "workspace_write", "danger_full_access"],
            "defaultModel": "sonnet",
            "models": [
                {"id": "sonnet", "label": "Sonnet", "tier": "standard", "defaultForTier": true, "supportedEffortLevels": []},
                {"id": "haiku", "label": "Haiku", "tier": "fast", "defaultForTier": true, "supportedEffortLevels": []}
            ]
        }))
        .unwrap()
    }
    fn start(&self, _: StartRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        Err(BridgeError::Invalid("start without start_chat".into()))
    }
    fn resume(&self, _: ResumeRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        Err(BridgeError::Invalid("resume without resume_chat".into()))
    }
    fn start_chat(&self, _: StartRequest<'_>, tools: Option<&ToolLaunch>) -> Result<StartedAdapter, BridgeError> {
        self.0.lock().unwrap().push(("start", false, tools.cloned()));
        Err(BridgeError::Invalid("recorded start".into()))
    }
    fn resume_chat(&self, request: ResumeRequest<'_>, tools: Option<&ToolLaunch>) -> Result<StartedAdapter, BridgeError> {
        self.0.lock().unwrap().push(("resume", request.fork, tools.cloned()));
        Err(BridgeError::Invalid("recorded resume".into()))
    }
    fn supports_native_resume(&self) -> bool {
        true
    }
    fn supports_native_fork(&self) -> bool {
        true
    }
    fn normalize(&self, _: &Value) -> Vec<agent::NormalizedEvent> {
        vec![]
    }
}

fn fixture() -> (tempfile::TempDir, Arc<BridgeCore>, Calls) {
    let dir = tempfile::tempdir().unwrap();
    let mut core = BridgeCore::for_tests(dir.path());
    let calls = Calls::default();
    let mut registry = AdapterRegistry::empty();
    registry.register(Box::new(Recorder(calls.clone()))).unwrap();
    core.adapter_registry = Arc::new(registry);
    (dir, Arc::new(core), calls)
}

/// Save a choice that leaves out `github`.
fn choose(core: &BridgeCore, session_id: &str) {
    let choice: ChatToolSelection = serde_json::from_value(json!({"harness": "claude", "serversOff": ["github"]})).unwrap();
    tool_selection::write_checked(&core.db.lock().unwrap(), session_id, 0, &choice).unwrap();
}

/// Start a session and give each launch that reached the adapter.
fn launches(core: &Arc<BridgeCore>, calls: &Calls, session_id: &str) -> Vec<Launch> {
    let error = start_chat(core, session_id.to_owned()).unwrap_err().to_string();
    assert!(error.contains("recorded"), "{session_id}: {error}");
    std::mem::take(&mut *calls.lock().unwrap())
}

fn chosen(launch: &Launch) -> bool {
    launch.2.as_ref().is_some_and(|tools| tools.servers_off == ["github"] && tools.revision == 1)
}

fn workflow_session(core: &BridgeCore, data_dir: &Path) -> &'static str {
    use crate::workflow_trigger::{self, ClaimOutcome, Occasion};
    use crate::workflows::test_support;
    let db = core.db.lock().unwrap();
    let workflow = test_support::save_approved(&db, data_dir, &test_support::definition());
    let claimed = match workflow_trigger::claim(&db, &workflow.id, &Occasion::manual(&workflow.id, "r1"), false, Utc::now()).unwrap() {
        ClaimOutcome::Claimed(claimed) => claimed,
        other => panic!("{other:?}"),
    };
    db.execute(
        "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind,depth) VALUES('run-session',NULL,'claude','Workflow','working','reported','workflow',0)",
        [],
    )
    .unwrap();
    assert!(workflow_trigger::transition(&db, &claimed.run.id, &claimed.lease_owner, wire::WorkflowRunStatus::Running, Some("run-session"), Utc::now()).unwrap());
    "run-session"
}

#[test]
fn workers_briefings_and_workflows_launch_without_a_tool_launch() {
    let (dir, core, calls) = fixture();
    // The control: a top-level chat with a choice launches with it.
    let chat = core.create_chat_id(&Harness::Claude, None, None).unwrap();
    choose(&core, &chat);
    let recorded = launches(&core, &calls, &chat);
    assert!(recorded.len() == 1 && recorded[0].0 == "start" && chosen(&recorded[0]), "{recorded:?}");

    // A child session and a worker each hold a row, and neither reads it.
    {
        let db = core.db.lock().unwrap();
        db.execute(
            "INSERT INTO sessions(id,harness,label,status,metric_source,kind,parent_session_id,depth) VALUES('child','claude','Child','idle','estimated','direct',?1,1)",
            params![chat],
        )
        .unwrap();
        db.execute("INSERT INTO sessions(id,harness,label,status,metric_source,kind,depth) VALUES('worker','claude','Worker','idle','estimated','direct',0)", []).unwrap();
        db.execute(
            "INSERT INTO worker_runtime(session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,result_status,updated_at) VALUES('worker',?1,'working','research','key','pending','now')",
            params![chat],
        )
        .unwrap();
    }
    for session in ["child", "worker"] {
        choose(&core, session);
        assert_eq!(launches(&core, &calls, session), [("start", false, None)], "{session}");
    }

    // A workflow run launches under its briefing policy, with no tool launch.
    let run = workflow_session(&core, dir.path());
    choose(&core, run);
    let policy = crate::workflow_run::policy_for_session(&core.db.lock().unwrap(), run).unwrap();
    assert!(policy.is_some(), "the run launches under a briefing policy");
    assert_eq!(launches(&core, &calls, run), [("start", false, None)]);
}

#[test]
fn a_fork_and_an_aside_launch_without_a_tool_launch() {
    let (_dir, core, calls) = fixture();
    let parent = core.create_chat_id(&Harness::Claude, None, None).unwrap();
    choose(&core, &parent);
    let entry = {
        let db = core.db.lock().unwrap();
        session_forest::SessionForest::new(&db)
            .append(&parent, session_forest::EntryKind::AssistantMessage, json!({"text": "common"}))
            .unwrap()
    };
    let (fork, _, _) = core.fork_session(&parent, &entry.id, None, None, None, "shared").unwrap();
    let (aside, _, native_fork) = core.create_aside_chat_id(&parent, &Harness::Claude, None, None).unwrap();
    assert!(!native_fork);
    for child in [&fork, &aside] {
        assert_eq!(tool_selection::read(&core.db.lock().unwrap(), child).unwrap(), None);
        let recorded = launches(&core, &calls, child);
        assert!(!recorded.is_empty() && recorded.iter().all(|launch| launch.2.is_none()), "{recorded:?}");
    }
    // The parent keeps its choice and launches with it.
    let recorded = launches(&core, &calls, &parent);
    assert!(!recorded.is_empty() && recorded.iter().all(chosen), "{recorded:?}");

    // A native fork of a stored thread carries no tool launch. A native
    // resume of the parent carries its choice.
    {
        let db = core.db.lock().unwrap();
        db.execute("UPDATE sessions SET provider_session_id='thread-1' WHERE id=?1", [&aside]).unwrap();
        restoration::set_head_state(&db, &aside, RestorationMode::NativeFork, ResumeEligibility::Native, Some("thread-1")).unwrap();
        db.execute("UPDATE sessions SET provider_session_id='thread-0' WHERE id=?1", [&parent]).unwrap();
    }
    let recorded = launches(&core, &calls, &aside);
    assert_eq!(recorded.first(), Some(&("resume", true, None)), "{recorded:?}");
    assert!(recorded.iter().all(|launch| launch.2.is_none()), "{recorded:?}");
    let recorded = launches(&core, &calls, &parent);
    assert_eq!(recorded.first().map(|launch| (launch.0, launch.1)), Some(("resume", false)), "{recorded:?}");
    assert!(recorded.iter().all(chosen), "{recorded:?}");
}

// --- a changed choice at the next turn -------------------------------------------

/// What one launch of [`Live`] asked for.
#[derive(Debug, Clone)]
struct Asked {
    verb: &'static str,
    provider: Option<String>,
    tools: Option<ToolLaunch>,
    instructions: Option<String>,
}

/// The record of the [`Live`] harnesses of one test, and the errors of their
/// next launches. A launch has the number of its entry in `asked`.
#[derive(Default)]
struct Script {
    asked: Vec<Asked>,
    resume_error: Option<String>,
    start_error: Option<String>,
    sent: Vec<(usize, String)>,
    stopped: Vec<usize>,
    /// The output of the next runtime ends at once, as for a process exit.
    output_ends: bool,
    /// A launch with a Codex skills part reports that it dropped the part,
    /// as for a config with a `skills` key of the user.
    drops_codex_skills: bool,
    /// Each usage seed: the harness, the provider id and the stored payload.
    seeds: Vec<(&'static str, String, Value)>,
}

type Shared = Arc<Mutex<Script>>;

/// A harness whose launches succeed with a runtime that has no process of
/// its own. A resume keeps the provider id that it asked for. A start gets
/// `thread-<number>`.
struct Live {
    id: &'static str,
    script: Shared,
}

impl Live {
    fn launch(&self, verb: &'static str, provider: Option<&str>, instructions: Option<&str>, tools: Option<&ToolLaunch>) -> Result<StartedAdapter, BridgeError> {
        let mut script = self.script.lock().unwrap();
        script.asked.push(Asked { verb, provider: provider.map(str::to_owned), tools: tools.cloned(), instructions: instructions.map(str::to_owned) });
        let number = script.asked.len();
        let error = if verb == "start" { script.start_error.clone() } else { script.resume_error.clone() };
        if let Some(error) = error {
            return Err(BridgeError::Invalid(error));
        }
        let thread = match (verb, provider) {
            ("resume", Some(provider)) => provider.to_owned(),
            _ => format!("thread-{number}"),
        };
        let reader: Box<dyn std::io::BufRead + Send> =
            if script.output_ends { Box::new(std::io::Cursor::new(Vec::new())) } else { Box::new(std::io::BufReader::new(Idle)) };
        Ok(StartedAdapter {
            runtime: Box::new(LiveRuntime { number, thread, turn: Arc::default(), script: self.script.clone() }),
            reader,
            startup_messages: vec![],
            skills_dropped: script.drops_codex_skills && tools.is_some_and(ToolLaunch::has_codex_skills),
        })
    }
}

impl HarnessAdapter for Live {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn descriptor(&self) -> AdapterDescriptor {
        serde_json::from_value(json!({
            "id": self.id, "label": self.id, "available": true, "authState": "unknown",
            "capabilities": ["messages", "briefings"],
            "sandboxModes": ["read_only", "workspace_write", "danger_full_access"],
            "defaultModel": "standard",
            "models": [
                {"id": "standard", "label": "Standard", "tier": "standard", "defaultForTier": true, "supportedEffortLevels": []},
                {"id": "fast", "label": "Fast", "tier": "fast", "defaultForTier": true, "supportedEffortLevels": []}
            ]
        }))
        .unwrap()
    }
    fn start(&self, _: StartRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        Err(BridgeError::Invalid("start without start_chat".into()))
    }
    fn resume(&self, _: ResumeRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        Err(BridgeError::Invalid("resume without resume_chat".into()))
    }
    fn start_chat(&self, request: StartRequest<'_>, tools: Option<&ToolLaunch>) -> Result<StartedAdapter, BridgeError> {
        self.launch("start", None, request.instructions, tools)
    }
    fn resume_chat(&self, request: ResumeRequest<'_>, tools: Option<&ToolLaunch>) -> Result<StartedAdapter, BridgeError> {
        let verb = if request.fork { "fork" } else { "resume" };
        self.launch(verb, Some(request.provider_session_id), request.instructions, tools)
    }
    fn supports_native_resume(&self) -> bool {
        true
    }
    fn supports_native_fork(&self) -> bool {
        true
    }
    fn normalize(&self, value: &Value) -> Vec<agent::NormalizedEvent> {
        if self.id == "claude" {
            agent::normalize_claude_message(value)
        } else {
            vec![]
        }
    }
    fn seed_usage_totals(&self, provider_session_id: &str, usage_event: &Value) {
        self.script.lock().unwrap().seeds.push((self.id, provider_session_id.to_owned(), usage_event.clone()));
    }
}

struct LiveRuntime {
    number: usize,
    thread: String,
    turn: Arc<Mutex<Option<String>>>,
    script: Shared,
}

impl crate::adapters::AdapterRuntime for LiveRuntime {
    /// The pid of the test: a launch records the identity of a real process.
    fn process_id(&self) -> u32 {
        std::process::id()
    }
    fn provider_session_id(&self) -> &str {
        &self.thread
    }
    fn current_turn(&self) -> Arc<Mutex<Option<String>>> {
        Arc::clone(&self.turn)
    }
    fn send_turn(&self, text: &str) -> Result<(), BridgeError> {
        self.script.lock().unwrap().sent.push((self.number, text.to_owned()));
        Ok(())
    }
    fn interrupt(&self) -> Result<(), BridgeError> {
        Ok(())
    }
    fn respond(&self, _: Value, _: &str) -> Result<(), BridgeError> {
        Ok(())
    }
    fn stop(&mut self, _: crate::adapters::ShutdownReason) {
        self.script.lock().unwrap().stopped.push(self.number);
    }
}

/// The output of a runtime that writes nothing. Its reader never exits, so
/// an old reader cannot remove the runtime of the next launch, which has the
/// same pid in a test.
struct Idle;

impl std::io::Read for Idle {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        loop {
            std::thread::park();
        }
    }
}

fn live() -> (tempfile::TempDir, Arc<BridgeCore>, Shared) {
    let dir = tempfile::tempdir().unwrap();
    let mut core = BridgeCore::for_tests(dir.path());
    let script = Shared::default();
    let mut registry = AdapterRegistry::empty();
    for id in ["claude", "codex", "opencode"] {
        registry.register(Box::new(Live { id, script: script.clone() })).unwrap();
    }
    core.adapter_registry = Arc::new(registry);
    (dir, Arc::new(core), script)
}

/// A top-level chat on `harness`. With a provider id, its next launch is a
/// native resume of that thread.
fn chat_on(core: &BridgeCore, harness: &str, provider: Option<&str>) -> String {
    let parsed = match harness {
        "claude" => Harness::Claude,
        "codex" => Harness::Codex,
        _ => Harness::OpenCode,
    };
    let chat = core.create_chat_id(&parsed, None, None).unwrap();
    if let Some(provider) = provider {
        core.db.lock().unwrap().execute("UPDATE sessions SET provider_session_id=?2 WHERE id=?1", params![chat, provider]).unwrap();
    }
    chat
}

/// Save the next revision of a choice that leaves out `servers`.
fn save_off(core: &BridgeCore, chat: &str, servers: &[&str]) -> u32 {
    let db = core.db.lock().unwrap();
    let harness: String = db.query_row("SELECT harness FROM sessions WHERE id=?1", [chat], |row| row.get(0)).unwrap();
    let choice: ChatToolSelection = serde_json::from_value(json!({"harness": harness, "serversOff": servers})).unwrap();
    let stored = tool_selection::read(&db, chat).unwrap().map_or(0, |row| row.revision);
    tool_selection::write_checked(&db, chat, stored, &choice).unwrap().revision
}

fn asked(script: &Shared) -> Vec<Asked> {
    std::mem::take(&mut script.lock().unwrap().asked)
}

/// The verb and the requested provider id of each launch.
fn verbs(launched: &[Asked]) -> Vec<(&'static str, Option<&str>)> {
    launched.iter().map(|launch| (launch.verb, launch.provider.as_deref())).collect()
}

fn revision_of(launch: &Asked) -> Option<(u32, Vec<String>)> {
    launch.tools.as_ref().map(|tools| (tools.revision, tools.servers_off.clone()))
}

/// The payloads of the change events of a chat, oldest first, without the
/// `_bridge` keys that the forest adds to each entry.
fn change_events(core: &BridgeCore, chat: &str) -> Vec<Value> {
    let db = core.db.lock().unwrap();
    let mut statement = db.prepare("SELECT payload FROM session_entries WHERE session_id=?1 AND kind=?2 ORDER BY sequence").unwrap();
    let payloads = statement.query_map(params![chat, tool_selection::CHANGED_KIND], |row| row.get::<_, String>(0)).unwrap();
    payloads
        .map(|payload| {
            let mut value: Value = serde_json::from_str(&payload.unwrap()).unwrap();
            value.as_object_mut().unwrap().retain(|key, _| !key.starts_with("_bridge"));
            value
        })
        .collect()
}

/// The status and the provider id of a chat.
fn row_of(core: &BridgeCore, chat: &str) -> (String, Option<String>) {
    core.db
        .lock()
        .unwrap()
        .query_row("SELECT status,provider_session_id FROM sessions WHERE id=?1", [chat], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
}

fn entry_count(core: &BridgeCore, chat: &str, kind: &str) -> i64 {
    core.db
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM session_entries WHERE session_id=?1 AND kind=?2", params![chat, kind], |row| row.get(0))
        .unwrap()
}

fn warm_thread(core: &BridgeCore, chat: &str) -> Option<String> {
    core.adapters.lock().unwrap().get(chat).map(|runtime| runtime.provider_session_id().to_owned())
}

/// Give one Claude frame to the frame handler of the chat.
fn frame(core: &Arc<BridgeCore>, chat: &str, value: Value) {
    handle_agent_value(core, chat, &Arc::new(Mutex::new(None)), &value);
}

fn claude_init(session: &str) -> Value {
    json!({"type": "system", "subtype": "init", "session_id": session})
}

fn good_turn() -> Value {
    json!({"type": "result", "subtype": "success", "result": "done"})
}

fn failed_turn() -> Value {
    json!({"type": "result", "subtype": "error_during_execution", "is_error": true})
}

/// The hash of the effective set that leaves out `github` on `harness`.
fn github_hash(core: &BridgeCore, chat: &str, harness: &str) -> String {
    let row = tool_selection::read(&core.db.lock().unwrap(), chat).unwrap().unwrap();
    let launch = ToolLaunch::from_row(&row);
    assert_eq!(launch.harness, harness);
    launch.effective_hash
}

#[test]
fn a_warm_chat_with_the_same_revision_is_reused() {
    let (_dir, core, script) = live();
    let chat = chat_on(&core, "codex", None);
    assert_eq!(save_off(&core, &chat, &["github"]), 1);
    start_chat(&core, chat.clone()).unwrap();
    assert_eq!(tool_selection::applied_revision(&core, &chat), 1);
    assert!(!tool_selection::stale(&core, &chat));

    // A second start and a send find the runtime of the same revision.
    start_chat(&core, chat.clone()).unwrap();
    submit_input(&core, chat.clone(), "hello".into()).unwrap();
    let launched = asked(&script);
    assert_eq!(verbs(&launched), [("start", None)]);
    assert_eq!(revision_of(&launched[0]), Some((1, vec!["github".to_owned()])));
    {
        let record = script.lock().unwrap();
        assert!(record.stopped.is_empty(), "{:?}", record.stopped);
        assert_eq!(record.sent.len(), 1, "{:?}", record.sent);
        assert!(record.sent[0].0 == 1 && record.sent[0].1.contains("hello"), "{:?}", record.sent);
    }

    // A chat with no choice is not stale either.
    let plain = chat_on(&core, "codex", None);
    start_chat(&core, plain.clone()).unwrap();
    start_chat(&core, plain.clone()).unwrap();
    assert_eq!(verbs(&asked(&script)), [("start", None)]);
    assert!(!tool_selection::stale(&core, &plain));

    // A runtime with no entry (a `start_session` launch) applies no choice,
    // so the stored revision 1 relaunches it.
    tool_selection::forget_applied(&core, &chat);
    start_chat(&core, chat.clone()).unwrap();
    assert_eq!(verbs(&asked(&script)), [("resume", Some("thread-1"))]);
    assert_eq!(tool_selection::applied_revision(&core, &chat), 1);
}

#[test]
fn the_exit_of_a_runtime_removes_its_applied_entry() {
    let (_dir, core, script) = live();
    let chat = chat_on(&core, "codex", None);
    save_off(&core, &chat, &["github"]);
    script.lock().unwrap().output_ends = true;
    start_chat(&core, chat.clone()).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while core.adapters.lock().unwrap().contains_key(&chat) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(!core.adapters.lock().unwrap().contains_key(&chat), "the reader exit removes the runtime");
    assert!(!core.applied_tools.lock().unwrap().contains_key(&chat), "the entry of the gone runtime stays");

    // The exit of an older launch keeps the entry of a newer one.
    script.lock().unwrap().output_ends = false;
    start_chat(&core, chat.clone()).unwrap();
    let started_at = core.applied_tools.lock().unwrap()[&chat].started_at.clone();
    tool_selection::forget_applied_launch(&core, &chat, "an older launch");
    assert_eq!(tool_selection::applied_revision(&core, &chat), 1);
    tool_selection::forget_applied_launch(&core, &chat, &started_at);
    assert!(!core.applied_tools.lock().unwrap().contains_key(&chat));
}

#[test]
fn a_changed_revision_relaunches_a_warm_chat_at_the_next_send() {
    let (_dir, core, script) = live();
    let chat = chat_on(&core, "codex", None);
    start_chat(&core, chat.clone()).unwrap();
    assert_eq!(tool_selection::applied_revision(&core, &chat), 0);
    assert_eq!(save_off(&core, &chat, &["github"]), 1);
    assert!(tool_selection::stale(&core, &chat));

    submit_input(&core, chat.clone(), "hello".into()).unwrap();
    let launched = asked(&script);
    // The send resumes the thread of the first launch with revision 1.
    assert_eq!(verbs(&launched), [("start", None), ("resume", Some("thread-1"))]);
    assert_eq!(revision_of(&launched[0]), None);
    assert_eq!(revision_of(&launched[1]), Some((1, vec!["github".to_owned()])));
    let sent = {
        let script = script.lock().unwrap();
        assert_eq!(script.stopped, [1], "the old runtime stops");
        script.sent.clone()
    };
    assert!(sent.len() == 1 && sent[0].0 == 2 && sent[0].1.contains("hello"), "the new runtime takes the send: {sent:?}");
    assert_eq!(tool_selection::applied_revision(&core, &chat), 1);
    assert!(!tool_selection::stale(&core, &chat));
    assert_eq!(warm_thread(&core, &chat).as_deref(), Some("thread-1"));
    assert_eq!(row_of(&core, &chat).1.as_deref(), Some("thread-1"));
}

#[test]
fn a_changed_revision_relaunches_through_start_chat() {
    let (_dir, core, script) = live();
    let chat = chat_on(&core, "opencode", None);
    assert_eq!(save_off(&core, &chat, &["github"]), 1);
    start_chat(&core, chat.clone()).unwrap();
    assert_eq!(save_off(&core, &chat, &["github", "linear"]), 2);
    start_chat(&core, chat.clone()).unwrap();

    let launched = asked(&script);
    assert_eq!(verbs(&launched), [("start", None), ("resume", Some("thread-1"))]);
    assert_eq!(revision_of(&launched[0]), Some((1, vec!["github".to_owned()])));
    assert_eq!(revision_of(&launched[1]), Some((2, vec!["github".to_owned(), "linear".to_owned()])));
    assert_eq!(script.lock().unwrap().stopped, [1]);
    assert_eq!(tool_selection::applied_revision(&core, &chat), 2);
    assert_eq!(warm_thread(&core, &chat).as_deref(), Some("thread-1"));

    // The relaunched runtime runs the stored revision, so the next start reuses it.
    start_chat(&core, chat.clone()).unwrap();
    assert!(asked(&script).is_empty());
    assert_eq!(script.lock().unwrap().stopped, [1]);
}

#[test]
fn a_changed_selection_resumes_natively_with_the_same_provider_session_id() {
    let (_dir, core, script) = live();
    for harness in ["claude", "codex", "opencode"] {
        let chat = chat_on(&core, harness, Some("thread-0"));
        save_off(&core, &chat, &["github"]);
        start_chat(&core, chat.clone()).unwrap();
        let launched = asked(&script);
        assert_eq!(verbs(&launched), [("resume", Some("thread-0"))], "{harness}");
        assert_eq!(revision_of(&launched[0]), Some((1, vec!["github".to_owned()])), "{harness}");
        assert_eq!(row_of(&core, &chat), ("ready".to_owned(), Some("thread-0".to_owned())), "{harness}");
        assert_eq!(warm_thread(&core, &chat).as_deref(), Some("thread-0"), "{harness}");
        let mode: String = core
            .db
            .lock()
            .unwrap()
            .query_row("SELECT restoration_mode FROM session_heads WHERE session_id=?1", [&chat], |row| row.get(0))
            .unwrap();
        assert_eq!(mode, RestorationMode::Native.as_str(), "{harness}");
    }
}

#[test]
fn a_failed_resume_with_a_changed_selection_keeps_the_provider_session_id_and_does_not_fork_or_start_fresh() {
    let (_dir, core, script) = live();
    script.lock().unwrap().resume_error = Some("the thread did not load".into());
    // The text names the cause and the way out, then the error of the harness.
    // The last good launch of each chat ran with each item on.
    let kept = "The harness did not resume this chat with the changed tools. Turn all on, then send again.";
    for harness in ["claude", "codex", "opencode"] {
        let chat = chat_on(&core, harness, Some("thread-0"));
        save_off(&core, &chat, &["github"]);
        let error = start_chat(&core, chat.clone()).unwrap_err().to_string();
        assert!(error.starts_with(kept), "{harness}: {error}");
        assert!(error.contains("the thread did not load"), "{harness}: {error}");
        let launched = asked(&script);
        assert_eq!(verbs(&launched), [("resume", Some("thread-0"))], "{harness}: one resume, no fork, no start");
        assert_eq!(row_of(&core, &chat), ("failed".to_owned(), Some("thread-0".to_owned())), "{harness}");
        assert_eq!(entry_count(&core, &chat, "session.resume_failed"), 1, "{harness}");
        assert!(change_events(&core, &chat).is_empty(), "{harness}");
        assert_eq!(warm_thread(&core, &chat), None, "{harness}");
        assert_eq!(tool_selection::applied_revision(&core, &chat), 0, "{harness}");

        // A send tries the same thread again and starts nothing else.
        let error = submit_input(&core, chat.clone(), "hello".into()).unwrap_err().to_string();
        assert!(error.contains("the thread did not load"), "{harness}: {error}");
        assert_eq!(verbs(&asked(&script)), [("resume", Some("thread-0"))], "{harness}");
        assert_eq!(row_of(&core, &chat).1.as_deref(), Some("thread-0"), "{harness}");
    }
    assert!(script.lock().unwrap().sent.is_empty());
}

#[test]
fn a_kept_thread_after_a_launch_with_an_item_off_names_only_the_earlier_set() {
    let (_dir, core, script) = live();
    // An all-on choice still differs from the set of the last good launch.
    let kept = "The harness did not resume this chat with the changed tools. Turn the tools back to the earlier set, then send again.";
    for harness in ["codex", "opencode"] {
        let chat = chat_on(&core, harness, Some("thread-0"));
        save_off(&core, &chat, &["github"]);
        start_chat(&core, chat.clone()).unwrap();
        assert_eq!(change_events(&core, &chat).len(), 1, "{harness}");
        core.adapters.lock().unwrap().remove(&chat);
        save_off(&core, &chat, &["github", "linear"]);
        asked(&script);
        script.lock().unwrap().resume_error = Some("the thread did not load".into());
        let error = start_chat(&core, chat.clone()).unwrap_err().to_string();
        assert!(error.starts_with(kept), "{harness}: {error}");
        assert!(error.contains("the thread did not load"), "{harness}: {error}");
        assert_eq!(verbs(&asked(&script)), [("resume", Some("thread-0"))], "{harness}");
        script.lock().unwrap().resume_error = None;
    }
}

#[test]
fn a_failed_resume_without_a_selection_change_keeps_the_existing_fallback() {
    let (_dir, core, script) = live();
    // A Codex chat with no choice.
    let plain = chat_on(&core, "codex", Some("thread-0"));
    // A Codex chat whose new revision keeps the set of its last event.
    let same = chat_on(&core, "codex", Some("thread-0"));
    save_off(&core, &same, &["github"]);
    start_chat(&core, same.clone()).unwrap();
    assert_eq!(change_events(&core, &same).len(), 1);
    core.adapters.lock().unwrap().remove(&same);
    assert_eq!(save_off(&core, &same, &["github"]), 2);
    asked(&script);

    script.lock().unwrap().resume_error = Some("the thread did not load".into());
    for chat in [&plain, &same] {
        start_chat(&core, chat.clone()).unwrap();
        let launched = asked(&script);
        assert_eq!(launched.iter().map(|launch| launch.verb).collect::<Vec<_>>(), ["resume", "start"], "{launched:?}");
        let thread = row_of(&core, chat).1.unwrap();
        assert!(thread.starts_with("thread-") && thread != "thread-0", "the fallback starts a new thread: {thread}");
        assert_eq!(row_of(&core, chat).0, "ready");
        assert_eq!(entry_count(&core, chat, "session.resume_failed"), 1);
    }
    assert_eq!(change_events(&core, &same).len(), 1, "an unchanged set writes no event");
}

#[test]
fn a_dropped_codex_skills_part_writes_no_claim_and_keeps_the_fallback() {
    let (_dir, core, script) = live();
    let chat = chat_on(&core, "codex", Some("thread-0"));
    let stored_hash = {
        let db = core.db.lock().unwrap();
        let choice: ChatToolSelection = serde_json::from_value(json!({"harness": "codex", "codexSkillsOff": ["review"]})).unwrap();
        let row = tool_selection::write_checked(&db, &chat, 0, &choice).unwrap();
        let stored_hash = ToolLaunch::from_row(&row).effective_hash;
        // An earlier launch applied the whole choice.
        let payload = json!({"revision": 1, "harness": "codex", "effectiveHash": stored_hash, "codexSkillsOff": ["review"]});
        crate::session_forest::SessionForest::new(&db)
            .append(&chat, crate::session_forest::EntryKind::ToolSelectionChanged, payload)
            .unwrap();
        stored_hash
    };
    // The config of this launch has a `skills` key of the user, so the
    // launch drops the skills part. The person did not change the set, so
    // the failed resume keeps the fallback.
    {
        let mut script = script.lock().unwrap();
        script.drops_codex_skills = true;
        script.resume_error = Some("the thread did not load".into());
    }
    start_chat(&core, chat.clone()).unwrap();
    let launched = asked(&script);
    assert_eq!(launched.iter().map(|launch| launch.verb).collect::<Vec<_>>(), ["resume", "start"], "{launched:?}");
    assert!(launched.iter().all(|launch| launch.tools.as_ref().is_some_and(|tools| tools.codex_skills_off == ["review"])), "{launched:?}");
    assert_eq!(row_of(&core, &chat).0, "ready");
    // The event names the set that the launch applied, not the stored choice.
    let events = change_events(&core, &chat);
    let last = events.last().unwrap();
    assert_eq!(last["codexSkillsOff"], json!([]));
    assert_eq!(last["effectiveHash"], json!(tool_selection::all_on_hash("codex")));
    assert_ne!(last["effectiveHash"], json!(stored_hash));
}

#[test]
fn a_dropped_codex_skills_part_writes_no_claim_and_the_next_send_does_not_relaunch() {
    let (_dir, core, script) = live();
    script.lock().unwrap().drops_codex_skills = true;
    let save = |chat: &str, choice: Value| {
        let db = core.db.lock().unwrap();
        let choice: ChatToolSelection = serde_json::from_value(choice).unwrap();
        ToolLaunch::from_row(&tool_selection::write_checked(&db, chat, 0, &choice).unwrap())
    };
    let chat = chat_on(&core, "codex", None);
    let stored = save(&chat, json!({"harness": "codex", "serversOff": ["github"], "codexSkillsOff": ["review"]}));
    start_chat(&core, chat.clone()).unwrap();
    let launched = asked(&script);
    assert_eq!(verbs(&launched), [("start", None)]);
    assert_eq!(launched[0].tools.as_ref().unwrap().codex_skills_off, ["review"], "the launch gets the skills part");

    // The launch reported the drop, so the event and the entry name the set that it applied.
    let applied = stored.without_codex_skills();
    assert_ne!(applied.effective_hash, stored.effective_hash);
    let events = change_events(&core, &chat);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!((&events[0]["serversOff"], &events[0]["codexSkillsOff"]), (&json!(["github"]), &json!([])));
    assert_eq!(events[0]["effectiveHash"], json!(applied.effective_hash));
    let entry = core.applied_tools.lock().unwrap()[&chat].clone();
    assert!(entry.skills_dropped, "the entry records the drop for the tool picker");
    assert_eq!((entry.revision, entry.effective_hash), (1, applied.effective_hash));

    // The entry keeps the stored revision, so the next send reuses the runtime.
    assert!(!tool_selection::stale(&core, &chat));
    submit_input(&core, chat.clone(), "hello".into()).unwrap();
    assert!(asked(&script).is_empty(), "no relaunch");
    assert!(script.lock().unwrap().stopped.is_empty());

    // A choice that only turns a skill off makes no claim.
    let skills = chat_on(&core, "codex", None);
    save(&skills, json!({"harness": "codex", "codexSkillsOff": ["review"]}));
    start_chat(&core, skills.clone()).unwrap();
    assert_eq!(asked(&script).len(), 1);
    assert!(change_events(&core, &skills).is_empty());
    assert_eq!(core.applied_tools.lock().unwrap()[&skills].effective_hash, tool_selection::all_on_hash("codex"));
}

#[test]
fn a_removed_codex_key_fails_the_launch_with_the_key_in_the_message() {
    let (_dir, core, script) = live();
    let refused = "Codex closed its output. Provider process exit status: 1. Stderr tail:\nError: invalid transport in mcp_servers.github";
    let expected = "The saved tool choice names a Codex server that is no longer configured: github";
    {
        let mut script = script.lock().unwrap();
        script.start_error = Some(refused.into());
        script.resume_error = Some(refused.into());
    }
    // A fresh start and a native resume give the same message.
    let fresh = chat_on(&core, "codex", None);
    save_off(&core, &fresh, &["github"]);
    let resumed = chat_on(&core, "codex", Some("thread-0"));
    save_off(&core, &resumed, &["github"]);
    for chat in [&fresh, &resumed] {
        let error = start_chat(&core, chat.clone()).unwrap_err().to_string();
        assert!(error.contains(expected), "{error}");
        assert!(change_events(&core, chat).is_empty());
    }
    assert_eq!(verbs(&asked(&script)), [("start", None), ("resume", Some("thread-0"))]);
    assert_eq!(row_of(&core, &resumed).1.as_deref(), Some("thread-0"));

    // A key that the choice does not leave out, or a longer key, keeps the
    // text of Codex.
    for servers in [&["linear"][..], &["git"][..]] {
        let chat = chat_on(&core, "codex", None);
        save_off(&core, &chat, servers);
        let error = start_chat(&core, chat.clone()).unwrap_err().to_string();
        assert!(!error.contains(expected) && error.contains("invalid transport in mcp_servers.github"), "{servers:?}: {error}");
    }
    // The same text from OpenCode is not a Codex key.
    let other = chat_on(&core, "opencode", None);
    save_off(&core, &other, &["github"]);
    let error = start_chat(&core, other).unwrap_err().to_string();
    assert!(!error.contains(expected), "{error}");
}

#[test]
fn a_changed_selection_delivers_the_same_system_bytes() {
    let (_dir, core, script) = live();
    for harness in ["claude", "codex", "opencode"] {
        let chat = chat_on(&core, harness, None);
        start_chat(&core, chat.clone()).unwrap();
        save_off(&core, &chat, &["github"]);
        start_chat(&core, chat.clone()).unwrap();
        save_off(&core, &chat, &["github", "linear"]);
        submit_input(&core, chat.clone(), "hello".into()).unwrap();
        let launched = asked(&script);
        assert_eq!(launched.len(), 3, "{harness}: {launched:?}");
        assert_eq!(launched.iter().map(revision_of).filter(Option::is_some).count(), 2, "{harness}");
        let first = launched[0].instructions.clone().unwrap();
        assert!(!first.is_empty(), "{harness}");
        for launch in &launched[1..] {
            assert_eq!(launch.instructions.as_deref(), Some(first.as_str()), "{harness}: {launch:?}");
        }
    }
}

#[test]
fn the_prefix_event_follows_a_successful_changed_codex_or_opencode_launch() {
    let (_dir, core, script) = live();
    for harness in ["codex", "opencode"] {
        let chat = chat_on(&core, harness, Some("thread-0"));
        save_off(&core, &chat, &["github"]);
        let hash = github_hash(&core, &chat, harness);
        start_chat(&core, chat.clone()).unwrap();
        assert_eq!(verbs(&asked(&script)), [("resume", Some("thread-0"))]);
        assert_eq!(
            change_events(&core, &chat),
            [json!({
                "revision": 1, "harness": harness, "effectiveHash": hash,
                "serversOff": ["github"], "codexAppsOff": false, "codexSkillsOff": [],
                "claudeSkills": "all", "providerSessionId": "thread-0", "prefixRewritten": true
            })],
            "{harness}"
        );
        // The event commits with the row of the launch, and the warm entry
        // needs no turn.
        let applied = core.applied_tools.lock().unwrap().get(&chat).cloned().unwrap();
        assert!(applied.confirmed && applied.pending.is_none(), "{harness}: {applied:?}");
        assert_eq!((applied.revision, applied.effective_hash), (1, hash), "{harness}");
        tool_selection::settle_after_turn(&core, &chat, false, None);
        assert_eq!(change_events(&core, &chat).len(), 1, "{harness}: a turn writes no second event");
    }
}

#[test]
fn the_prefix_event_follows_the_first_claude_turn_on_the_same_session() {
    let (_dir, core, script) = live();
    let chat = chat_on(&core, "claude", Some("thread-0"));
    save_off(&core, &chat, &["github"]);
    let hash = github_hash(&core, &chat, "claude");
    start_chat(&core, chat.clone()).unwrap();
    assert_eq!(verbs(&asked(&script)), [("resume", Some("thread-0"))]);
    // The launch alone proves nothing for Claude.
    assert!(change_events(&core, &chat).is_empty());
    assert!(!core.applied_tools.lock().unwrap()[&chat].confirmed);

    frame(&core, &chat, claude_init("thread-0"));
    assert!(change_events(&core, &chat).is_empty(), "the init alone is not a turn");
    frame(&core, &chat, failed_turn());
    assert!(change_events(&core, &chat).is_empty(), "a failed turn is not a proof");
    frame(&core, &chat, good_turn());
    assert_eq!(
        change_events(&core, &chat),
        [json!({
            "revision": 1, "harness": "claude", "effectiveHash": hash,
            "serversOff": ["github"], "codexAppsOff": false, "codexSkillsOff": [],
            "claudeSkills": "all", "providerSessionId": "thread-0", "prefixRewritten": true
        })]
    );
    let applied = core.applied_tools.lock().unwrap()[&chat].clone();
    assert!(applied.confirmed && applied.pending.is_none(), "{applied:?}");

    // The next good turn writes no second event.
    frame(&core, &chat, good_turn());
    assert_eq!(change_events(&core, &chat).len(), 1);
}

/// The statements of the connections that trace into it. Only one test
/// traces.
static TRACED: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn traced(statement: &str) {
    TRACED.lock().unwrap().push(statement.to_owned());
}

#[test]
fn a_claude_turn_settles_the_event_with_no_read_of_the_history() {
    let (_dir, core, _script) = live();
    let chat = chat_on(&core, "claude", Some("thread-0"));
    save_off(&core, &chat, &["github"]);
    start_chat(&core, chat.clone()).unwrap();
    core.db.lock().unwrap().trace(Some(traced));
    frame(&core, &chat, claude_init("thread-0"));
    frame(&core, &chat, good_turn());
    core.db.lock().unwrap().trace(None);
    let statements = std::mem::take(&mut *TRACED.lock().unwrap());
    assert!(!statements.is_empty());
    let reads: Vec<&String> = statements.iter().filter(|sql| sql.contains("$.data.subtype") || sql.contains("$.data.session_id")).collect();
    assert!(reads.is_empty(), "the frame path reads the history: {reads:#?}");
    assert_eq!(change_events(&core, &chat).len(), 1);
    assert!(core.applied_tools.lock().unwrap()[&chat].confirmed);
}

#[test]
fn a_turn_of_an_old_launch_does_not_settle_the_event_of_the_new_one() {
    let (_dir, core, script) = live();
    let chat = chat_on(&core, "claude", Some("thread-0"));
    let started_at = || -> String {
        core.db.lock().unwrap().query_row("SELECT started_at FROM sessions WHERE id=?1", [&chat], |row| row.get(0)).unwrap()
    };
    save_off(&core, &chat, &["github"]);
    start_chat(&core, chat.clone()).unwrap();
    let old = started_at();
    assert_eq!(save_off(&core, &chat, &["github", "linear"]), 2);
    start_chat(&core, chat.clone()).unwrap();
    let new = started_at();
    assert_ne!(old, new);
    assert_eq!(verbs(&asked(&script)), [("resume", Some("thread-0")), ("resume", Some("thread-0"))]);
    let turn = Arc::new(Mutex::new(None));
    // An init of the old launch proves nothing for the new one.
    handle_agent_value_timed(&core, &chat, &turn, &claude_init("thread-0"), None, Some((&old, "thread-0")));
    let pending = |core: &BridgeCore| core.applied_tools.lock().unwrap()[&chat].pending.clone();
    assert_eq!(pending(&core).unwrap().init_session_id, None);
    handle_agent_value_timed(&core, &chat, &turn, &claude_init("thread-0"), None, Some((&new, "thread-0")));

    // The good turn of the old launch ends after the relaunch.
    tool_selection::settle_after_turn(&core, &chat, false, Some((&old, "thread-0")));
    assert!(change_events(&core, &chat).is_empty(), "an old launch writes no event");
    assert!(pending(&core).is_some(), "the event of the new launch stays in wait");

    tool_selection::settle_after_turn(&core, &chat, false, Some((&new, "thread-0")));
    let events = change_events(&core, &chat);
    assert_eq!((events.len(), events[0]["revision"].clone()), (1, json!(2)));
    assert!(core.applied_tools.lock().unwrap()[&chat].confirmed);
}

#[test]
fn no_prefix_event_after_a_failed_launch_or_an_unchanged_set() {
    let (_dir, core, script) = live();
    // A failed launch writes no event and keeps no warm entry.
    let failed = chat_on(&core, "codex", None);
    save_off(&core, &failed, &["github"]);
    script.lock().unwrap().start_error = Some("Codex did not start".into());
    assert!(start_chat(&core, failed.clone()).is_err());
    assert!(change_events(&core, &failed).is_empty());
    assert!(!core.applied_tools.lock().unwrap().contains_key(&failed));
    // The next launch that succeeds writes the event.
    script.lock().unwrap().start_error = None;
    start_chat(&core, failed.clone()).unwrap();
    assert_eq!(change_events(&core, &failed).len(), 1);

    // A new revision of the same set relaunches the chat with no event.
    assert_eq!(save_off(&core, &failed, &["github"]), 2);
    start_chat(&core, failed.clone()).unwrap();
    assert_eq!(tool_selection::applied_revision(&core, &failed), 2);
    assert_eq!(change_events(&core, &failed).len(), 1);

    // A chat with no choice launches with the set that it always had.
    let plain = chat_on(&core, "opencode", None);
    start_chat(&core, plain.clone()).unwrap();
    assert!(change_events(&core, &plain).is_empty());

    // A Claude turn on another session id proves nothing, and the event of
    // that launch is dropped.
    let claude = chat_on(&core, "claude", Some("thread-0"));
    save_off(&core, &claude, &["github"]);
    start_chat(&core, claude.clone()).unwrap();
    frame(&core, &claude, claude_init("another-session"));
    frame(&core, &claude, good_turn());
    assert!(change_events(&core, &claude).is_empty());
    frame(&core, &claude, claude_init("thread-0"));
    frame(&core, &claude, good_turn());
    assert!(change_events(&core, &claude).is_empty(), "the dropped event does not come back");
    assert!(!core.applied_tools.lock().unwrap()[&claude].confirmed);
    asked(&script);
}

#[test]
fn the_first_launch_with_a_first_choice_records_the_event_without_a_prefix_rewrite() {
    let (_dir, core, script) = live();
    for harness in ["codex", "opencode"] {
        let chat = chat_on(&core, harness, None);
        save_off(&core, &chat, &["github"]);
        start_chat(&core, chat.clone()).unwrap();
        let launched = asked(&script);
        assert_eq!(verbs(&launched), [("start", None)], "{harness}");
        let thread = row_of(&core, &chat).1.unwrap();
        let events = change_events(&core, &chat);
        assert_eq!(events.len(), 1, "{harness}");
        assert_eq!(events[0]["prefixRewritten"], false, "{harness}");
        assert_eq!(events[0]["providerSessionId"], thread.as_str(), "{harness}");
        assert_eq!(events[0]["revision"], 1, "{harness}");
    }

    // Claude records it at the first good turn of the new session.
    let chat = chat_on(&core, "claude", None);
    save_off(&core, &chat, &["github"]);
    start_chat(&core, chat.clone()).unwrap();
    let thread = row_of(&core, &chat).1.unwrap();
    frame(&core, &chat, claude_init(&thread));
    frame(&core, &chat, good_turn());
    let events = change_events(&core, &chat);
    assert_eq!(events.len(), 1);
    assert_eq!((events[0]["prefixRewritten"].clone(), events[0]["providerSessionId"].clone()), (json!(false), json!(thread)));

    // A later change of the same chat rewrites the prefix.
    save_off(&core, &chat, &["github", "linear"]);
    start_chat(&core, chat.clone()).unwrap();
    frame(&core, &chat, claude_init(&thread));
    frame(&core, &chat, good_turn());
    let events = change_events(&core, &chat);
    assert_eq!(events.len(), 2);
    assert_eq!(events[1]["prefixRewritten"], true);
}

/// Store one `usage.updated` entry of `harness` for a chat, with a total cost.
fn store_usage(core: &BridgeCore, chat: &str, harness: &str, cost: f64) {
    let mut event = agent::NormalizedEvent::new("usage.updated");
    event.data = json!({"usage": {"input_tokens": 1}, "totalCostUsd": cost, "modelUsage": {"m": {"inputTokens": 1, "costUSD": cost}}});
    store::session_event(&core.db.lock().unwrap(), chat, &event, &json!({"adapter": harness})).unwrap();
}

fn seeds(script: &Shared) -> Vec<(&'static str, String, Value)> {
    std::mem::take(&mut script.lock().unwrap().seeds)
}

#[test]
fn a_native_resume_seeds_the_claude_totals_from_the_last_usage_entry() {
    let (_dir, core, script) = live();
    let chat = chat_on(&core, "claude", Some("thread-0"));
    store_usage(&core, &chat, "claude", 0.25);
    store_usage(&core, &chat, "claude", 0.5);
    // A later entry of another kind does not hide the last usage entry.
    store::session_event(&core.db.lock().unwrap(), &chat, &agent::NormalizedEvent::new("turn.completed"), &json!({"adapter": "claude"})).unwrap();
    start_chat(&core, chat.clone()).unwrap();
    assert_eq!(verbs(&asked(&script)), [("resume", Some("thread-0"))]);
    let seeded = seeds(&script);
    assert_eq!(seeded.len(), 1, "{seeded:?}");
    assert_eq!((seeded[0].0, seeded[0].1.as_str()), ("claude", "thread-0"));
    assert_eq!(seeded[0].2["data"]["totalCostUsd"], json!(0.5));
    assert_eq!(seeded[0].2["providerMeta"]["adapter"], "claude");

    // A native resume of a chat with no usage entry calls no seed.
    let empty = chat_on(&core, "claude", Some("thread-1"));
    start_chat(&core, empty).unwrap();
    assert_eq!(verbs(&asked(&script)), [("resume", Some("thread-1"))]);
    assert!(seeds(&script).is_empty());

    // A Codex chat and an OpenCode chat get no Claude seed.
    for harness in ["codex", "opencode"] {
        let other = chat_on(&core, harness, Some("thread-2"));
        store_usage(&core, &other, harness, 0.5);
        start_chat(&core, other).unwrap();
        assert_eq!(verbs(&asked(&script)), [("resume", Some("thread-2"))], "{harness}");
        assert!(seeds(&script).is_empty(), "{harness}");
    }
}

#[test]
fn a_fresh_claude_start_gets_no_seed() {
    let (_dir, core, script) = live();
    let chat = chat_on(&core, "claude", None);
    store_usage(&core, &chat, "claude", 0.5);
    start_chat(&core, chat).unwrap();
    assert_eq!(verbs(&asked(&script)), [("start", None)]);
    assert!(seeds(&script).is_empty());

    // A failed native resume falls back to a fresh start, which gets no seed.
    let failed = chat_on(&core, "claude", Some("thread-0"));
    store_usage(&core, &failed, "claude", 0.5);
    script.lock().unwrap().resume_error = Some("the session did not load".into());
    start_chat(&core, failed).unwrap();
    assert_eq!(asked(&script).iter().map(|launch| launch.verb).collect::<Vec<_>>(), ["resume", "start"]);
    assert!(seeds(&script).is_empty());
}

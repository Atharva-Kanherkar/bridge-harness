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

    // A child session and a worker each hold a row. The generic start refuses
    // both, so neither reaches the adapter with it.
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
        assert_eq!(start_chat(&core, session.to_owned()).unwrap_err().to_string(), WORKER_START_REFUSAL, "{session}");
        assert!(calls.lock().unwrap().is_empty(), "{session}");
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
    /// The buffered startup frames of the next launch.
    startup: Vec<Value>,
    /// The lines that the reader of the next launch reads before it waits.
    output: Vec<Value>,
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
        let lines: String = std::mem::take(&mut script.output).iter().map(|line| format!("{line}\n")).collect();
        let reader: Box<dyn std::io::BufRead + Send> = if script.output_ends {
            Box::new(std::io::Cursor::new(Vec::new()))
        } else {
            Box::new(std::io::BufReader::new(std::io::Read::chain(std::io::Cursor::new(lines.into_bytes()), Idle)))
        };
        Ok(StartedAdapter {
            runtime: Box::new(LiveRuntime { number, thread, turn: Arc::default(), script: self.script.clone() }),
            reader,
            startup_messages: std::mem::take(&mut script.startup),
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
    // A workspace chat and a worker launch through `start` and `resume`.
    fn start(&self, request: StartRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        self.launch("start", None, request.instructions, None)
    }
    fn resume(&self, request: ResumeRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        let verb = if request.fork { "fork" } else { "resume" };
        self.launch(verb, Some(request.provider_session_id), request.instructions, None)
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
        match self.id {
            "claude" => agent::normalize_claude_message(value),
            "codex" => agent::normalize_codex_message(value),
            _ => vec![],
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
    store_claude_usage(&core, &chat, "thread-0", 0.25);
    store_claude_usage(&core, &chat, "thread-0", 0.5);
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

const D_THREAD: &str = "D-thread";

/// A Codex usage frame of `thread` with the totals and the last request of a
/// turn: input, cached input and output tokens.
fn codex_usage(thread: &str, turn: &str, total: [i64; 3], last: [i64; 3]) -> Value {
    let breakdown = |[input, cached, output]: [i64; 3]| json!({
        "totalTokens": input + output, "inputTokens": input, "cachedInputTokens": cached,
        "cacheWriteInputTokens": 0, "outputTokens": output, "reasoningOutputTokens": 0
    });
    json!({"method": "thread/tokenUsage/updated", "params": {
        "threadId": thread, "turnId": turn,
        "tokenUsage": {"total": breakdown(total), "last": breakdown(last), "modelContextWindow": 475000}
    }})
}

/// The real usage frame of the second turn D2 of a Codex chat. Codex sent it
/// again after `thread/resume` in a new process.
fn d2_usage(thread: &str) -> Value {
    codex_usage(thread, "D2", [64142, 31872, 10], [32080, 31872, 5])
}

/// The real usage frame of the third turn D3, the first turn after the resume.
fn d3_usage(thread: &str) -> Value {
    codex_usage(thread, "D3", [96969, 63744, 15], [32827, 31872, 5])
}

fn codex_turn(method: &str, thread: &str, turn: &str) -> Value {
    json!({"method": method, "params": {"threadId": thread, "turn": {"id": turn, "status": "completed"}}})
}

/// The `usage.updated` entries, the ledger rows and the context readings of a chat.
fn usage_counts(core: &BridgeCore, chat: &str) -> (i64, i64, i64) {
    let db = core.db.lock().unwrap();
    let count = |sql: &str| db.query_row(sql, [chat], |row| row.get::<_, i64>(0)).unwrap();
    (
        count("SELECT COUNT(*) FROM session_entries WHERE session_id=?1 AND kind='usage.updated'"),
        count("SELECT COUNT(*) FROM usage_ledger WHERE session_id=?1"),
        count("SELECT COUNT(*) FROM context_readings WHERE session_id=?1"),
    )
}

/// A Codex chat on `thread` whose turn D2 recorded its usage in this daemon.
fn codex_chat_after_d2(core: &Arc<BridgeCore>, thread: &str) -> String {
    let chat = chat_on(core, "codex", Some(thread));
    for value in [codex_turn("turn/started", thread, "D2"), d2_usage(thread), codex_turn("turn/completed", thread, "D2")] {
        frame(core, &chat, value);
    }
    assert_eq!(usage_counts(core, &chat), (1, 1, 1));
    chat
}

/// Wait until the reader of a chat stored `count` entries of `kind`.
fn wait_for_entries(core: &BridgeCore, chat: &str, kind: &str, count: i64) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while entry_count(core, chat, kind) < count {
        assert!(std::time::Instant::now() < deadline, "the reader stored no {kind} entry");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn a_resumed_codex_thread_drops_the_replay_of_its_last_usage() {
    let (_dir, core, script) = live();
    // A daemon that still holds the old turn id, and a new daemon that holds
    // none. The replay is a buffered startup frame or a line of the reader.
    for (thread, new_daemon, buffered) in [("D-1", false, true), ("D-2", false, false), ("D-3", true, true), ("D-4", true, false)] {
        let chat = codex_chat_after_d2(&core, thread);
        let mut delegations = core.delegations.lock().unwrap();
        if new_daemon {
            delegations.last_turn_by_session.remove(&chat);
        } else {
            assert_eq!(delegations.last_turn_by_session.get(&chat).map(String::as_str), Some("D2"));
        }
        drop(delegations);
        {
            let mut script = script.lock().unwrap();
            if buffered {
                script.startup = vec![d2_usage(thread)];
            } else {
                script.output = vec![d2_usage(thread)];
            }
            // The next turn starts after the replay, so its entry shows that
            // the reader read the replay.
            script.output.push(codex_turn("turn/started", thread, "D3"));
        }
        start_chat(&core, chat.clone()).unwrap();
        assert_eq!(verbs(&asked(&script)), [("resume", Some(thread))]);
        wait_for_entries(&core, &chat, "turn.started", 2);
        assert_eq!(usage_counts(&core, &chat), (1, 1, 1), "{thread}");
    }
}

#[test]
fn the_first_usage_of_the_next_turn_records_after_a_codex_resume() {
    let (_dir, core, script) = live();
    let chat = codex_chat_after_d2(&core, D_THREAD);
    script.lock().unwrap().output = vec![
        d2_usage(D_THREAD),
        codex_turn("turn/started", D_THREAD, "D3"),
        d3_usage(D_THREAD),
        codex_turn("turn/completed", D_THREAD, "D3"),
    ];
    start_chat(&core, chat.clone()).unwrap();
    wait_for_entries(&core, &chat, "turn.completed", 2);
    assert_eq!(usage_counts(&core, &chat), (2, 2, 2));
    let db = core.db.lock().unwrap();
    let row: (Option<String>, i64, i64, i64, i64) = db
        .query_row(
            "SELECT turn_id,input_tokens,cache_read_tokens,uncached_input_tokens,output_tokens FROM usage_ledger WHERE session_id=?1 ORDER BY id DESC LIMIT 1",
            [&chat],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .unwrap();
    assert_eq!(row, (Some("D3".into()), 32827, 31872, 955, 5));
    let reading: (Option<String>, i64, i64) = db
        .query_row(
            "SELECT turn_id,used_tokens,window_tokens FROM context_readings WHERE session_id=?1 ORDER BY id DESC LIMIT 1",
            [&chat],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(reading, (Some("D3".into()), 32832, 475000));
    let entry: String = db
        .query_row(
            "SELECT payload FROM session_entries WHERE session_id=?1 AND kind='usage.updated' ORDER BY sequence DESC LIMIT 1",
            [&chat],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(serde_json::from_str::<Value>(&entry).unwrap()["data"]["turnId"], "D3");
}

#[test]
fn a_codex_usage_frame_that_differs_from_the_seed_still_records() {
    let (_dir, core, script) = live();
    let chat = codex_chat_after_d2(&core, D_THREAD);
    // The same turn id with other figures, and the same figures with another turn id.
    let other_figures = codex_usage(D_THREAD, "D2", [64142, 31872, 11], [32080, 31872, 6]);
    let mut other_turn = d2_usage(D_THREAD);
    other_turn["params"]["turnId"] = json!("D1");
    script.lock().unwrap().output = vec![
        d2_usage(D_THREAD),
        other_figures,
        other_turn,
        // After the first turn starts, the seed is gone and an equal frame records.
        codex_turn("turn/started", D_THREAD, "D3"),
        d2_usage(D_THREAD),
        codex_turn("turn/completed", D_THREAD, "D3"),
    ];
    start_chat(&core, chat.clone()).unwrap();
    wait_for_entries(&core, &chat, "turn.completed", 2);
    // D2 before the resume, then the two frames that differ and the late equal frame.
    assert_eq!(usage_counts(&core, &chat), (4, 4, 4));
    let db = core.db.lock().unwrap();
    let mut entries = db.prepare("SELECT payload FROM session_entries WHERE session_id=?1 AND kind='usage.updated' ORDER BY sequence").unwrap();
    let entries: Vec<(String, i64)> = entries
        .query_map([&chat], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|payload| {
            let data = serde_json::from_str::<Value>(&payload.unwrap()).unwrap()["data"].clone();
            (data["turnId"].as_str().unwrap().to_owned(), data["tokenUsage"]["last"]["outputTokens"].as_i64().unwrap())
        })
        .collect();
    assert_eq!(entries, [("D2".into(), 5), ("D2".into(), 6), ("D1".into(), 5), ("D2".into(), 5)]);
    let mut ledger = db.prepare("SELECT output_tokens FROM usage_ledger WHERE session_id=?1 ORDER BY id").unwrap();
    let ledger: Vec<i64> = ledger.query_map([&chat], |row| row.get(0)).unwrap().map(Result::unwrap).collect();
    assert_eq!(ledger, [5, 6, 5, 5]);
}

#[test]
fn a_new_codex_thread_and_a_chat_with_no_stored_usage_get_no_seed() {
    let (_dir, core, script) = live();
    // The chat has stored Codex usage, but its next launch starts a new thread.
    let chat = codex_chat_after_d2(&core, D_THREAD);
    core.db.lock().unwrap().execute("UPDATE sessions SET provider_session_id=NULL WHERE id=?1", [&chat]).unwrap();
    script.lock().unwrap().output = vec![d2_usage("thread-1"), codex_turn("turn/started", "thread-1", "E1")];
    start_chat(&core, chat.clone()).unwrap();
    assert_eq!(verbs(&asked(&script)), [("start", None)]);
    wait_for_entries(&core, &chat, "turn.started", 2);
    assert_eq!(usage_counts(&core, &chat), (2, 2, 2));

    // A native resume of a chat with no stored usage.
    let empty = chat_on(&core, "codex", Some("E-thread"));
    script.lock().unwrap().output = vec![d2_usage("E-thread"), codex_turn("turn/started", "E-thread", "E1")];
    start_chat(&core, empty.clone()).unwrap();
    assert_eq!(verbs(&asked(&script)), [("resume", Some("E-thread"))]);
    wait_for_entries(&core, &empty, "turn.started", 1);
    assert_eq!(usage_counts(&core, &empty), (1, 1, 1));
}

/// A workspace with a stopped workspace chat on `harness`. Its next
/// `start_session` resumes the provider session `thread` natively.
fn workspace_chat(core: &BridgeCore, data_dir: &Path, harness: &str, thread: &str) -> String {
    let path = data_dir.join("workspace");
    let db = core.db.lock().unwrap();
    db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo',?1,'now')", params![path.to_string_lossy()]).unwrap();
    db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Task','bridge/task',?1,'stopped','now')", params![path.to_string_lossy()]).unwrap();
    db.execute(
        "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,depth,kind,provider_session_id) VALUES('workspace-chat','w',?1,'Orchestrator','stopped','reported',0,'orchestrator',?2)",
        params![harness, thread],
    )
    .unwrap();
    "workspace-chat".into()
}

#[test]
fn a_native_resume_of_a_workspace_chat_seeds_the_claude_totals() {
    let (dir, core, script) = live();
    let chat = workspace_chat(&core, dir.path(), "claude", "thread-0");
    store_claude_usage(&core, &chat, "thread-0", 0.5);
    start_session(&core, "w".into(), Some(Harness::Claude), None).unwrap();
    assert_eq!(verbs(&asked(&script)), [("resume", Some("thread-0"))]);
    let seeded = seeds(&script);
    assert_eq!(seeded.len(), 1, "{seeded:?}");
    assert_eq!((seeded[0].0, seeded[0].1.as_str()), ("claude", "thread-0"));
    assert_eq!(seeded[0].2["data"]["totalCostUsd"], json!(0.5));
}

#[test]
fn a_native_resume_of_a_workspace_chat_drops_the_codex_replay() {
    let (dir, core, script) = live();
    let chat = workspace_chat(&core, dir.path(), "codex", D_THREAD);
    for value in [codex_turn("turn/started", D_THREAD, "D2"), d2_usage(D_THREAD), codex_turn("turn/completed", D_THREAD, "D2")] {
        frame(&core, &chat, value);
    }
    assert_eq!(usage_counts(&core, &chat), (1, 1, 1));
    {
        let mut script = script.lock().unwrap();
        script.startup = vec![d2_usage(D_THREAD)];
        script.output = vec![d2_usage(D_THREAD), codex_turn("turn/started", D_THREAD, "D3")];
    }
    start_session(&core, "w".into(), Some(Harness::Codex), None).unwrap();
    assert_eq!(verbs(&asked(&script)), [("resume", Some(D_THREAD))]);
    wait_for_entries(&core, &chat, "turn.started", 2);
    assert_eq!(usage_counts(&core, &chat), (1, 1, 1));
}

/// A stopped Claude worker with no runtime. The next delegation resumes its
/// provider session natively.
#[test]
fn a_cold_worker_that_resumes_natively_gets_the_seed() {
    let (dir, core, script) = live();
    let request = delegation::DelegationRequest {
        schema_version: delegation::SCHEMA_VERSION,
        role: delegation::WorkerRole::Research,
        objective: "Inspect the next task".into(),
        acceptance_criteria: vec!["Return evidence for the new objective".into()],
        known_facts: vec![],
        decisions: vec![], evidence_ids: vec![], relevant_files: vec![],
        owned_paths: vec![], write_mode: delegation::WriteMode::ReadOnly,
        capability_tier: crate::model::CapabilityTier::Standard,
        effort: delegation::Effort::Medium, network_access: false,
        writable_output_paths: vec![], verification: vec![],
        output_contract: delegation::OutputContract::ResearchResult,
        harness: Some("claude".into()), model: Some("standard".into()),
    };
    // A read-only worker records a baseline of the tracked files first.
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    for args in [&["init", "-q"][..], &["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "commit", "-q", "--allow-empty", "-m", "start"]] {
        assert!(crate::git::git_command(&repo).args(args).status().unwrap().success());
    }
    {
        let db = core.db.lock().unwrap();
        db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo',?1,'now')", params![repo.to_string_lossy()]).unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Task','bridge/task',?1,'working','now')", params![repo.to_string_lossy()]).unwrap();
        db.execute_batch(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,depth,kind)
                VALUES('parent','w','claude','Parent','working','reported',0,'orchestrator');
             INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id,depth,model,provider_session_id)
                VALUES('worker','w','claude','Research','stopped','reported','parent',1,'standard','worker-thread');
             INSERT INTO worker_leases(session_id,workspace_id,role,capability_tier,task_family,owned_paths,write_mode,lease_status,created_at,updated_at)
                VALUES('worker','w','research','standard','research','[]','readOnly','checkpointed','now','now');
             INSERT INTO worker_runtime(session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,result_status,updated_at)
                VALUES('worker','parent','stopped','research','key','reported','now');",
        )
        .unwrap();
    }
    store_claude_usage(&core, "worker", "worker-thread", 0.5);
    match launch_worker_outcome(&core, "parent", "turn-1", &request, false) {
        WorkerLaunchOutcome::Launched(id) => assert_eq!(id, "worker"),
        _ => {
            let rejected: String = core.db.lock().unwrap().query_row("SELECT COALESCE(group_concat(payload, '; '),'none') FROM session_entries WHERE kind='delegation.rejected'", [], |row| row.get(0)).unwrap();
            panic!("the cold worker did not launch: {rejected}");
        }
    }
    assert_eq!(verbs(&asked(&script)), [("resume", Some("worker-thread"))]);
    let seeded = seeds(&script);
    assert_eq!(seeded.len(), 1, "{seeded:?}");
    assert_eq!((seeded[0].0, seeded[0].1.as_str()), ("claude", "worker-thread"));
}

/// Store the Claude `init` entry of the provider session `thread`.
fn store_claude_init(core: &BridgeCore, chat: &str, thread: &str) {
    let db = core.db.lock().unwrap();
    for event in agent::normalize_claude_message(&claude_init(thread)) {
        store::session_event(&db, chat, &event, &json!({"adapter": "claude"})).unwrap();
    }
}

/// Store one Claude `usage.updated` entry with the saved totals of the
/// provider session `thread`, as the normalizer writes it now.
fn store_claude_usage(core: &BridgeCore, chat: &str, thread: &str, cost: f64) {
    let model_usage = json!({"m": {"inputTokens": 1, "costUSD": cost}});
    let mut event = agent::NormalizedEvent::new("usage.updated");
    event.data = json!({
        "usage": {"input_tokens": 1}, "totalCostUsd": cost, "modelUsage": model_usage,
        "cumulative": {"sessionId": thread, "totalCostUsd": cost, "modelUsage": model_usage}
    });
    store::session_event(&core.db.lock().unwrap(), chat, &event, &json!({"adapter": "claude"})).unwrap();
}

/// Store one Codex `usage.updated` entry with the raw usage of `thread`.
fn store_codex_usage(core: &BridgeCore, chat: &str, thread: Option<&str>, turn: &str) {
    let mut event = agent::NormalizedEvent::new("usage.updated");
    event.data = codex_usage(thread.unwrap_or_default(), turn, [9, 0, 9], [9, 0, 9])["params"].clone();
    if thread.is_none() {
        event.data.as_object_mut().unwrap().remove("threadId");
    }
    store::session_event(&core.db.lock().unwrap(), chat, &event, &json!({"adapter": "codex"})).unwrap();
}

#[test]
fn a_newer_usage_entry_of_another_thread_does_not_hide_the_codex_seed() {
    let (_dir, core, script) = live();
    let chat = codex_chat_after_d2(&core, D_THREAD);
    // An entry of another thread, and an entry that names no thread.
    store_codex_usage(&core, &chat, Some("other-thread"), "X1");
    store_codex_usage(&core, &chat, None, "X2");
    script.lock().unwrap().output = vec![d2_usage(D_THREAD), codex_turn("turn/started", D_THREAD, "D3")];
    start_chat(&core, chat.clone()).unwrap();
    assert_eq!(verbs(&asked(&script)), [("resume", Some(D_THREAD))]);
    wait_for_entries(&core, &chat, "turn.started", 2);
    assert_eq!(usage_counts(&core, &chat), (3, 1, 1));
}

#[test]
fn a_newer_usage_entry_of_another_adapter_does_not_hide_the_seed() {
    let (_dir, core, script) = live();
    let claude = chat_on(&core, "claude", Some("thread-0"));
    store_claude_usage(&core, &claude, "thread-0", 0.5);
    store_codex_usage(&core, &claude, Some("thread-0"), "X1");
    start_chat(&core, claude).unwrap();
    assert_eq!(verbs(&asked(&script)), [("resume", Some("thread-0"))]);
    let seeded = seeds(&script);
    assert_eq!(seeded.len(), 1, "{seeded:?}");
    assert_eq!(seeded[0].2["data"]["cumulative"]["totalCostUsd"], json!(0.5));

    let codex = codex_chat_after_d2(&core, D_THREAD);
    store_claude_usage(&core, &codex, D_THREAD, 0.5);
    script.lock().unwrap().output = vec![d2_usage(D_THREAD), codex_turn("turn/started", D_THREAD, "D3")];
    start_chat(&core, codex.clone()).unwrap();
    assert_eq!(verbs(&asked(&script)), [("resume", Some(D_THREAD))]);
    wait_for_entries(&core, &codex, "turn.started", 2);
    assert_eq!(usage_counts(&core, &codex), (2, 1, 1));
}

#[test]
fn an_old_claude_usage_entry_of_another_provider_session_gives_no_seed() {
    let (_dir, core, script) = live();
    // Session A left an entry with no saved totals. Session B started in the
    // same chat and stored no usage, and now B resumes.
    let chat = chat_on(&core, "claude", Some("thread-B"));
    store_claude_init(&core, &chat, "thread-A");
    store_usage(&core, &chat, "claude", 0.5);
    store_claude_init(&core, &chat, "thread-B");
    start_chat(&core, chat).unwrap();
    assert_eq!(verbs(&asked(&script)), [("resume", Some("thread-B"))]);
    assert!(seeds(&script).is_empty());

    // An old entry with no `init` entry before it names no session.
    let unnamed = chat_on(&core, "claude", Some("thread-C"));
    store_usage(&core, &unnamed, "claude", 0.5);
    start_chat(&core, unnamed).unwrap();
    assert_eq!(verbs(&asked(&script)), [("resume", Some("thread-C"))]);
    assert!(seeds(&script).is_empty());
}

#[test]
fn an_old_claude_usage_entry_of_the_same_provider_session_still_gives_the_seed() {
    let (_dir, core, script) = live();
    let chat = chat_on(&core, "claude", Some("thread-A"));
    store_claude_init(&core, &chat, "thread-A");
    store_usage(&core, &chat, "claude", 0.5);
    // A newer entry of another adapter comes first, so the read goes past it.
    store_codex_usage(&core, &chat, None, "X1");
    start_chat(&core, chat).unwrap();
    assert_eq!(verbs(&asked(&script)), [("resume", Some("thread-A"))]);
    let seeded = seeds(&script);
    assert_eq!(seeded.len(), 1, "{seeded:?}");
    assert_eq!((seeded[0].0, seeded[0].1.as_str()), ("claude", "thread-A"));
    assert_eq!(seeded[0].2["data"]["totalCostUsd"], json!(0.5));
}

// --- the worker start guards ---------------------------------------------------

/// A stopped Claude worker of parent, as launch_worker writes it. The
/// insert names no kind, so the row is an orchestrator, at depth 1, with a
/// runtime row.
fn bridge_worker(core: &BridgeCore, id: &str, parent: &str, thread: &str) -> String {
    let db = core.db.lock().unwrap();
    db.execute(
        "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,model,provider_session_id,parent_session_id,depth)
         SELECT ?1,workspace_id,'claude','Research','stopped','reported','standard',?3,id,1 FROM sessions WHERE id=?2",
        params![id, parent, thread],
    )
    .unwrap();
    db.execute(
        "INSERT INTO worker_runtime(session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,result_status,updated_at) VALUES(?1,?2,'stopped','research','key','reported','now')",
        params![id, parent],
    )
    .unwrap();
    id.to_owned()
}

/// The depth, the parent and the kind of a session.
fn lineage(core: &BridgeCore, session: &str) -> (i64, Option<String>, String) {
    core.db
        .lock()
        .unwrap()
        .query_row("SELECT depth,parent_session_id,kind FROM sessions WHERE id=?1", [session], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
}

#[test]
fn start_chat_refuses_a_bridge_worker_and_launches_nothing() {
    let (dir, core, script) = live();
    let chat = workspace_chat(&core, dir.path(), "claude", "thread-0");
    let worker = bridge_worker(&core, "worker", &chat, "worker-thread");
    let before = lineage(&core, &worker);
    assert_eq!(before, (1, Some(chat.clone()), "orchestrator".to_owned()));

    // The RPC body of sessions/start_chat, which any daemon client can send.
    let error = crate::api::start_chat(&core, worker.clone()).unwrap_err().to_string();
    assert_eq!(error, WORKER_START_REFUSAL);
    assert!(asked(&script).is_empty(), "nothing reached the launcher");
    assert!(warm_thread(&core, &worker).is_none());
    assert_eq!(lineage(&core, &worker), before, "the worker keeps its depth, parent and kind");
    assert_eq!(row_of(&core, &worker), ("stopped".to_owned(), Some("worker-thread".to_owned())));
}

#[test]
fn start_chat_refuses_a_live_worker_without_replacing_its_process() {
    let (dir, core, script) = live();
    let chat = workspace_chat(&core, dir.path(), "claude", "thread-0");
    let worker = bridge_worker(&core, "worker", &chat, "worker-thread");
    core.db.lock().unwrap().execute("UPDATE sessions SET status='working' WHERE id=?1", [&worker]).unwrap();
    let runtime = LiveRuntime { number: 1, thread: "worker-thread".into(), turn: Arc::default(), script: script.clone() };
    core.adapters.lock().unwrap().insert(worker.clone(), Box::new(runtime));
    let before = lineage(&core, &worker);

    // No prompt record matches the live process, so a start past the guard
    // would stop it and launch another.
    let error = start_chat(&core, worker.clone()).unwrap_err().to_string();
    assert_eq!(error, WORKER_START_REFUSAL);
    {
        let script = script.lock().unwrap();
        assert!(script.asked.is_empty(), "nothing reached the launcher: {:?}", verbs(&script.asked));
        assert!(script.stopped.is_empty(), "the live process was stopped: {:?}", script.stopped);
    }
    assert_eq!(warm_thread(&core, &worker).as_deref(), Some("worker-thread"), "the same process stays in place");
    assert_eq!(lineage(&core, &worker), before);
    assert_eq!(row_of(&core, &worker).0, "working");
}

#[test]
fn start_chat_still_reaches_the_launcher_for_a_top_level_chat() {
    let (dir, core, script) = live();
    // A worker row is an orchestrator too. The control is a top-level
    // orchestrator of the same workspace and harness, and a direct chat.
    let chat = workspace_chat(&core, dir.path(), "claude", "thread-0");
    let worker = bridge_worker(&core, "worker", &chat, "worker-thread");
    assert_eq!(start_chat(&core, worker.clone()).unwrap_err().to_string(), WORKER_START_REFUSAL);
    start_chat(&core, chat.clone()).unwrap();
    let direct = chat_on(&core, "claude", None);
    start_chat(&core, direct.clone()).unwrap();

    assert_eq!(verbs(&asked(&script)), [("resume", Some("thread-0")), ("start", None)]);
    assert_eq!(warm_thread(&core, &chat).as_deref(), Some("thread-0"));
    assert!(warm_thread(&core, &direct).is_some());
    assert!(warm_thread(&core, &worker).is_none());
}

#[test]
fn start_session_reuse_skips_workers_and_harness_subagents() {
    let (dir, core, script) = live();
    let chat = workspace_chat(&core, dir.path(), "claude", "thread-0");

    // A newer Bridge worker of the same workspace and harness.
    let worker = bridge_worker(&core, "worker", &chat, "worker-thread");
    let worker_lineage = lineage(&core, &worker);
    start_session(&core, "w".into(), Some(Harness::Claude), None).unwrap();
    assert_eq!(verbs(&asked(&script)), [("resume", Some("thread-0"))], "the top-level chat is reused, not the worker");
    assert!(warm_thread(&core, &worker).is_none());
    assert_eq!(lineage(&core, &worker), worker_lineage);

    // A newer worker row with no parent on the session: an old row from before
    // the guards. Only its worker runtime keeps it out of the reuse.
    {
        let db = core.db.lock().unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,depth,kind,provider_session_id) VALUES('orphan-worker','w','claude','Research','stopped','reported',0,'direct','orphan-thread')", []).unwrap();
        db.execute("INSERT INTO worker_runtime(session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,result_status,updated_at) VALUES('orphan-worker',?1,'stopped','research','key','reported','now')", params![chat]).unwrap();
    }
    start_session(&core, "w".into(), Some(Harness::Claude), None).unwrap();
    let launched = asked(&script);
    assert!(launched.iter().all(|launch| launch.provider.as_deref() == Some("thread-0")), "{:?}", verbs(&launched));
    assert!(warm_thread(&core, "orphan-worker").is_none());

    // A newer harness subagent of the chat.
    core.db
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind,depth,parent_session_id,provider_session_id) VALUES('subagent','w','claude','Subagent','working','estimated',?1,1,?2,'subagent-thread')",
            params![crate::harness_subagents::KIND, chat],
        )
        .unwrap();
    start_session(&core, "w".into(), Some(Harness::Claude), None).unwrap();
    let launched = asked(&script);
    assert!(launched.iter().all(|launch| launch.provider.as_deref() == Some("thread-0")), "{:?}", verbs(&launched));
    assert!(warm_thread(&core, "subagent").is_none());
    assert_eq!(lineage(&core, "subagent"), (1, Some(chat.clone()), crate::harness_subagents::KIND.to_owned()));

    // A Codex orchestrator with a Claude worker: the workspace has no
    // reusable Claude session. This phase does not assert the start result:
    // the path for a new session fails at is_top_level_chat, before a launch.
    // The worker must stay as it was.
    {
        let db = core.db.lock().unwrap();
        db.execute(
            "INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w2','p','Lima','Other','bridge/other',?1,'stopped','now')",
            params![dir.path().join("other").to_string_lossy()],
        )
        .unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,depth,kind) VALUES('codex-chat','w2','codex','Orchestrator','stopped','reported',0,'orchestrator')", []).unwrap();
    }
    let claude_worker = bridge_worker(&core, "claude-worker", "codex-chat", "claude-worker-thread");
    start_session(&core, "w2".into(), Some(Harness::Claude), None).unwrap();
    assert_eq!(verbs(&asked(&script)), [("start", None)], "a new session starts, the worker is not reused");
    let started = new_session_of(&core, "w2", "claude", &[&claude_worker]);
    assert!(warm_thread(&core, &started).is_some());
    assert!(warm_thread(&core, &claude_worker).is_none());
    assert_eq!(lineage(&core, &claude_worker), (1, Some("codex-chat".to_owned()), "orchestrator".to_owned()));
}

#[test]
fn start_session_reuses_only_direct_and_orchestrator_chats() {
    const OTHER_KINDS: [&str; 3] = ["workflow", "briefing", "imported"];

    for kind in [Some("direct"), Some("orchestrator"), None] {
        let (dir, core, script) = live();
        let harness = if kind.is_some() { "claude" } else { "codex" };
        let chat = workspace_chat(&core, dir.path(), harness, "chat-thread");
        {
            let db = core.db.lock().unwrap();
            if let Some(kind) = kind {
                db.execute("UPDATE sessions SET kind=?1 WHERE id=?2", params![kind, chat])
                    .unwrap();
            }
            for other in OTHER_KINDS {
                db.execute(
                    "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,depth,kind)
                     VALUES(?1,'w','claude',?1,'stopped','reported',0,?1)",
                    [other],
                )
                .unwrap();
            }
        }

        start_session(&core, "w".into(), Some(Harness::Claude), None).unwrap();
        if kind.is_some() {
            assert_eq!(verbs(&asked(&script)), [("resume", Some("chat-thread"))]);
            assert_eq!(warm_thread(&core, &chat).as_deref(), Some("chat-thread"));
        } else {
            assert_eq!(verbs(&asked(&script)), [("start", None)]);
            let started = new_session_of(&core, "w", "claude", &OTHER_KINDS);
            assert_eq!(lineage(&core, &started), (0, None, "orchestrator".into()));
            assert!(warm_thread(&core, &started).is_some());
        }
        for other in OTHER_KINDS {
            assert_eq!(row_of(&core, other), ("stopped".into(), None), "{other}");
            assert!(warm_thread(&core, other).is_none(), "{other}");
        }
    }
}

/// The one session of the workspace and harness that is not in `known`.
fn new_session_of(core: &BridgeCore, workspace: &str, harness: &str, known: &[&str]) -> String {
    let db = core.db.lock().unwrap();
    let mut statement = db.prepare("SELECT id FROM sessions WHERE workspace_id=?1 AND harness=?2 ORDER BY rowid").unwrap();
    let ids: Vec<String> = statement
        .query_map(params![workspace, harness], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .filter(|id: &String| !known.contains(&id.as_str()))
        .collect();
    assert_eq!(ids.len(), 1, "{ids:?}");
    ids[0].clone()
}

#[test]
fn start_session_starts_a_new_session_when_none_is_reusable() {
    let (dir, core, script) = live();
    // The workspace has a Codex chat only, so no Claude session is reusable.
    let codex_chat = workspace_chat(&core, dir.path(), "codex", "codex-thread");
    start_session(&core, "w".into(), Some(Harness::Claude), None).unwrap();
    assert_eq!(verbs(&asked(&script)), [("start", None)]);
    let started = new_session_of(&core, "w", "claude", &[]);
    assert_ne!(started, codex_chat);
    assert_eq!(lineage(&core, &started), (0, None, "orchestrator".to_owned()));
    assert_eq!(row_of(&core, &started).1.as_deref(), Some("thread-1"));
    assert_eq!(warm_thread(&core, &started).as_deref(), Some("thread-1"));
    assert!(warm_thread(&core, &codex_chat).is_none());
}

mod off_trace;

/// The off guarantee of `testing/feat-early-compaction.md`. Each later commit
/// of early compaction keeps this trace equal while the switch is off.
#[test]
fn with_the_switch_off_send_frame_and_manual_compaction_traces_are_unchanged() {
    off_trace::check(off_trace::Switch::Absent);
    off_trace::check(off_trace::Switch::SavedOff);
}

// Early compaction: the due mark, the single dispatch path, the adapter
// command, the attempt record and the off guarantee. The world and the
// frames are in `early.rs`.
mod early;

use early::*;
use off_trace::{claude_turn_with_auto_compaction, codex_compaction_start, codex_turn_completed, CLAUDE_CHAT, CODEX_CHAT};

#[test]
fn a_completed_direct_chat_turn_is_marked_due() -> TestResult {
    for chat in FIRING_CHATS {
        let world = World::new()?;
        world.switch(true)?;
        let before = Utc::now();
        let turn = world.turn(chat, LARGE)?;
        let after = Utc::now();
        let mark = world.mark(chat).ok_or("no due mark")?;
        let (started_at, provider_session_id) = world.launch(chat)?;
        assert_eq!(mark.launch, Some(crate::early_compaction::Launch { started_at, provider_session_id }), "{chat}");
        assert_eq!(mark.harness, if chat == CLAUDE_CHAT { "claude" } else { "codex" });
        assert!(mark.completed_at >= before && mark.completed_at <= after, "{chat}");
        if chat == CODEX_CHAT {
            assert_eq!(mark.turn_id.as_deref(), Some(turn.as_str()), "a Codex reading binds by this turn id");
        }
        // The mark waits for the sweep: the frame path dispatches nothing.
        assert!(world.compacts(chat).is_empty() && world.attempts(chat)?.is_empty(), "{chat}");
    }
    Ok(())
}

#[test]
fn a_failed_turn_is_not_marked() -> TestResult {
    for chat in FIRING_CHATS {
        assert!(marked_after(chat, Ending::Completed)?, "{chat}: the control turn");
        assert!(!marked_after(chat, Ending::Failed)?, "{chat}");
    }
    Ok(())
}

#[test]
fn a_provider_error_is_not_marked() -> TestResult {
    for chat in FIRING_CHATS {
        assert!(marked_after(chat, Ending::Completed)?, "{chat}: the control turn");
        assert!(!marked_after(chat, Ending::ProviderError)?, "{chat}");
    }
    Ok(())
}

#[test]
fn a_prompt_control_turn_is_not_marked() -> TestResult {
    for chat in FIRING_CHATS {
        assert!(marked_after(chat, Ending::Completed)?, "{chat}: the control turn");
        let world = World::new()?;
        world.switch(true)?;
        world.turn_ending(chat, Ending::PromptControl, LARGE)?;
        let recorded = world.count("SELECT COUNT(*) FROM events WHERE kind='prompt_mutation.control_turn' AND entity_id=?1", params![chat]);
        assert_eq!(recorded?, 1, "{chat}: the frame path saw a prompt control turn");
        assert!(world.mark(chat).is_none(), "{chat}");
    }
    Ok(())
}

#[test]
fn a_checkpoint_turn_is_not_marked() -> TestResult {
    for chat in FIRING_CHATS {
        assert!(marked_after(chat, Ending::Completed)?, "{chat}: the control turn");
        let world = World::new()?;
        world.switch(true)?;
        world.send(chat, "Read notes.txt and summarise it.")?;
        world.sql("UPDATE sessions SET status='checkpointing' WHERE id=?1", params![chat])?;
        let (_, frames) = turn_frames(chat, 1, Ending::Completed, LARGE);
        world.feed(chat, &frames)?;
        assert!(world.mark(chat).is_none(), "{chat}");
    }
    Ok(())
}

#[test]
fn the_turn_of_an_open_attempt_is_not_marked() -> TestResult {
    for chat in FIRING_CHATS {
        let world = fired(chat)?;
        assert!(world.mark(chat).is_none(), "{chat}: the fire took the mark");
        world.turn_end_without_boundary(chat)?;
        assert!(world.mark(chat).is_none(), "{chat}: the compaction turn gave a mark");
        assert_eq!(world.attempt(chat)?.outcome.as_deref(), Some("ended_without_boundary"), "{chat}");
    }
    Ok(())
}

#[test]
fn the_turn_end_of_a_compaction_makes_no_due_mark() -> TestResult {
    for chat in FIRING_CHATS {
        // An early compaction.
        let world = fired(chat)?;
        world.compaction(chat)?;
        assert_eq!(world.attempt(chat)?.outcome.as_deref(), Some("compacted"), "{chat}");
        assert!(world.mark(chat).is_none(), "{chat}: early");
        // A manual `/compact`. Its boundary also takes the mark of the turn
        // before it: that mark no longer tells the size of the window.
        let (world, _) = marked(chat)?;
        world.manual_compaction(chat)?;
        assert!(world.mark(chat).is_none(), "{chat}: manual");
        // A compaction that the harness starts inside a turn.
        let world = World::new()?;
        world.switch(true)?;
        world.send(chat, "Reply with the single word ok.")?;
        let frames = if chat == CLAUDE_CHAT { claude_turn_with_auto_compaction() } else { codex_turn_with_auto_compaction("codex-turn-1", LARGE) };
        world.feed(chat, &frames)?;
        assert!(world.mark(chat).is_none(), "{chat}: harness");
        assert!(world.compacts(chat).is_empty() && world.attempts(chat)?.is_empty(), "{chat}");
    }
    Ok(())
}

#[test]
fn the_turn_after_a_compaction_is_marked_again() -> TestResult {
    for chat in FIRING_CHATS {
        let world = fired(chat)?;
        world.compaction(chat)?;
        assert!(world.mark(chat).is_none(), "{chat}");
        let turn = world.turn(chat, LARGE)?;
        let mark = world.mark(chat).ok_or("no mark after the compaction")?;
        if chat == CODEX_CHAT {
            assert_eq!(mark.turn_id, Some(turn));
        }
        let (world, _) = marked(chat)?;
        world.manual_compaction(chat)?;
        world.turn(chat, LARGE)?;
        assert!(world.mark(chat).is_some(), "{chat}: after a manual compaction");
    }
    Ok(())
}

#[test]
fn a_boundary_flag_of_an_old_launch_does_not_hide_a_turn_end() -> TestResult {
    for chat in FIRING_CHATS {
        let world = World::new()?;
        world.switch(true)?;
        // A manual `/compact` gets its boundary. Its process stops before its turn end.
        world.send(chat, "/compact")?;
        let (turn, item) = ("codex-turn-old", "codex-compaction-old");
        let mut frames = if chat == CLAUDE_CHAT { claude_compaction_start() } else { codex_compaction_start(3, turn, item) };
        frames.push(if chat == CLAUDE_CHAT { claude_boundary(Some(POST_TOKENS)) } else { codex_boundary(turn, item) });
        world.feed(chat, &frames)?;
        // The resume is a new launch. Its first turn ends as a normal turn.
        world.relaunch(chat)?;
        world.turn(chat, LARGE)?;
        assert!(world.mark(chat).is_some(), "{chat}: the flag of the old launch hid the turn end");
    }
    Ok(())
}

#[test]
fn a_fire_removes_the_due_mark() -> TestResult {
    for chat in FIRING_CHATS {
        let (world, _) = marked(chat)?;
        // Inside the idle delay the mark waits and stays.
        world.sweep_after(5);
        assert!(world.mark(chat).is_some() && world.compacts(chat).is_empty(), "{chat}");
        world.fire();
        assert_eq!(world.compacts(chat).len(), 1, "{chat}");
        assert!(world.mark(chat).is_none(), "{chat}");
        world.fire();
        assert_eq!(world.compacts(chat).len(), 1, "{chat}: a second sweep sends again");
    }
    Ok(())
}

#[test]
fn a_mark_that_goes_before_the_decision_does_not_fire() -> TestResult {
    for chat in FIRING_CHATS {
        // The boundary of a manual `/compact` takes the mark after the sweep
        // copied it and before the sweep decides.
        let (world, _) = marked(chat)?;
        let core = Arc::downgrade(&world.core);
        world.levers.on_support(move || {
            if let Some(core) = core.upgrade() {
                let launch = core.early_compaction.due_mark(chat).and_then(|mark| mark.launch);
                core.early_compaction.note_boundary(chat, launch.as_ref());
            }
        });
        world.fire();
        assert!(world.compacts(chat).is_empty() && world.attempts(chat)?.is_empty(), "{chat}: a removed mark fired");
        assert!(world.mark(chat).is_none(), "{chat}");
        // A newer turn end replaces the mark in the same window.
        let (world, _) = marked(chat)?;
        let core = Arc::downgrade(&world.core);
        world.levers.on_support(move || {
            if let Some(core) = core.upgrade() {
                if let Some(mut newer) = core.early_compaction.due_mark(chat) {
                    newer.turn_id = Some("a-newer-turn".to_owned());
                    newer.completed_at += chrono::TimeDelta::seconds(1);
                    core.early_compaction.accept(crate::early_compaction::Receipt::Due(newer));
                }
            }
        });
        world.fire();
        assert!(world.compacts(chat).is_empty() && world.attempts(chat)?.is_empty(), "{chat}: a replaced mark fired");
        let kept = world.mark(chat).ok_or("the newer mark went")?;
        assert_eq!(kept.turn_id.as_deref(), Some("a-newer-turn"), "{chat}");
    }
    Ok(())
}

#[test]
fn an_orchestrator_or_worker_turn_is_not_marked() -> TestResult {
    let changes = [
        "UPDATE sessions SET kind='orchestrator' WHERE id=?1",
        "UPDATE sessions SET depth=1,parent_session_id='opencode-chat' WHERE id=?1",
    ];
    for chat in FIRING_CHATS {
        for change in changes {
            let world = World::new()?;
            world.switch(true)?;
            world.send(chat, "Read notes.txt and summarise it.")?;
            world.sql(change, params![chat])?;
            let (_, frames) = turn_frames(chat, 1, Ending::Completed, LARGE);
            world.feed(chat, &frames)?;
            assert!(world.mark(chat).is_none(), "{chat}: {change}");
        }
    }
    Ok(())
}

#[test]
fn no_mark_with_the_switch_off() -> TestResult {
    for chat in FIRING_CHATS {
        let absent = World::new()?;
        absent.turn(chat, LARGE)?;
        assert!(absent.mark(chat).is_none(), "{chat}: switch absent");
        let saved_off = World::new()?;
        saved_off.switch(false)?;
        saved_off.turn(chat, LARGE)?;
        assert!(saved_off.mark(chat).is_none(), "{chat}: switch saved off");
        saved_off.switch(true)?;
        saved_off.turn(chat, LARGE)?;
        assert!(saved_off.mark(chat).is_some(), "{chat}: the control turn");
    }
    Ok(())
}

#[test]
fn with_the_switch_on_a_turn_frame_adds_no_sql() -> TestResult {
    for chat in FIRING_CHATS {
        let mut runs = Vec::new();
        for on in [false, true] {
            let world = World::new()?;
            world.switch(on)?;
            world.send(chat, "Read notes.txt and summarise it.")?;
            let (started_at, provider) = world.launch(chat)?;
            let (_, frames) = turn_frames(chat, 1, Ending::Completed, LARGE);
            let (fed, ran) = statements(&world.core, || world.feed_as(chat, (&started_at, &provider), &frames));
            fed?;
            runs.push((shapes(&ran)?, world.mark(chat).is_some()));
        }
        let (off, on) = (&runs[0], &runs[1]);
        assert!(!off.0.is_empty(), "{chat}: the trace caught the frame path");
        assert!(!off.1 && on.1, "{chat}: only the run with the switch on marks");
        assert_eq!(off.0, on.0, "{chat}: the due mark added SQL to the frame path");
    }
    Ok(())
}

#[test]
fn the_slash_command_and_the_early_trigger_share_forward_native_compaction() -> TestResult {
    for chat in FIRING_CHATS {
        let (manual, _) = marked(chat)?;
        manual.take_calls();
        manual.send(chat, "/compact")?;
        let (early, _) = marked(chat)?;
        early.take_calls();
        early.fire();
        let calls = early.take_calls();
        assert_eq!(calls, vec![json!({"session": chat, "call": "compact_native", "args": [null]})], "{chat}");
        assert_eq!(calls, manual.take_calls(), "{chat}: the same adapter call");
        assert_eq!(early.status(chat)?, "working", "{chat}: the same optimistic mark");
        assert_eq!(early.status(chat)?, manual.status(chat)?, "{chat}");
        assert_eq!(early.attempts(chat)?.len(), 1, "{chat}");
        assert!(manual.attempts(chat)?.is_empty(), "{chat}: a manual compaction writes no attempt");
    }
    Ok(())
}

#[test]
fn a_user_send_and_the_early_trigger_cannot_both_dispatch() -> TestResult {
    let text = "And list the line numbers.";
    for chat in FIRING_CHATS {
        // The send first: a sweep inside its adapter call finds the lease
        // taken and keeps the mark.
        let (world, _) = marked(chat)?;
        world.take_calls();
        let core = Arc::downgrade(&world.core);
        world.levers.on_send(move || {
            if let Some(core) = core.upgrade() {
                crate::early_compaction::sweep(&core, Utc::now() + chrono::TimeDelta::seconds(WARM_SECONDS));
            }
        });
        world.send(chat, text)?;
        assert!(world.compacts(chat).is_empty(), "{chat}: the sweep dispatched during a send");
        assert!(world.mark(chat).is_some(), "{chat}: a busy lease keeps the mark");
        assert!(lock(&world.levers.leased_sends).iter().all(|leased| *leased), "{chat}: a send ran without the lease");
        let calls = world.take_calls();
        assert!(!world.routes(chat, text, &calls)?.is_empty(), "{chat}: the send lost the text: {calls:?}");
        // The sweep first: a send that starts inside its adapter call waits
        // for the dispatch to finish.
        let (world, _) = marked(chat)?;
        world.take_calls();
        let race = Race::arm(&world, move |core| submit_input(&core, chat.to_owned(), text.to_owned()).is_ok());
        world.fire();
        let (waited, sent) = race.finish()?;
        assert!(waited && sent, "{chat}: waited {waited}, sent {sent}");
        let calls = world.take_calls();
        assert_eq!(calls.first().map(|call| call["call"].clone()), Some(json!("compact_native")), "{chat}: {calls:?}");
        assert_eq!(world.compacts(chat).len(), 0, "{chat}: the log was taken");
        assert!(!world.routes(chat, text, &calls)?.is_empty(), "{chat}: the send after the dispatch lost the text: {calls:?}");
    }
    Ok(())
}

/// The result, the adapter calls, the status and the routes of the text of
/// one send after a compaction that a person or the sweep asked for.
fn send_after_compaction(chat: &str, variant: Variant, early: bool) -> Outcome<(String, Vec<Value>, String, Vec<String>)> {
    let (world, _) = marked(chat)?;
    if early {
        world.fire();
    } else {
        world.send(chat, "/compact")?;
    }
    world.take_calls();
    let result = world.send_variant(chat, variant);
    let calls = world.take_calls();
    let routes = world.routes(chat, FOLLOW_UP, &calls)?;
    Ok((without_ids(&result)?, calls, world.status(chat)?, routes))
}

#[test]
fn a_send_after_an_early_dispatch_routes_as_after_a_manual_compact() -> TestResult {
    // The text reaches the runtime or the queue, or the caller gets an error
    // and keeps the text.
    let kept = |(result, _, _, routes): &(String, Vec<Value>, String, Vec<String>)| {
        !routes.is_empty() || result.starts_with("{\"error\":")
    };
    for chat in FIRING_CHATS {
        for variant in VARIANTS {
            let manual = send_after_compaction(chat, variant, false)?;
            let early = send_after_compaction(chat, variant, true)?;
            assert!(kept(&manual), "{chat}: {variant:?}: the send after a manual compact lost the text: {manual:?}");
            assert!(kept(&early), "{chat}: {variant:?}: the send after an early dispatch lost the text: {early:?}");
            assert_eq!(early, manual, "{chat}: {variant:?}");
        }
    }
    Ok(())
}

#[test]
fn an_opt_out_saved_before_the_reservation_wins() -> TestResult {
    for chat in FIRING_CHATS {
        let (world, _) = marked(chat)?;
        world.opt_out(chat, true)?;
        world.fire();
        assert!(world.compacts(chat).is_empty() && world.attempts(chat)?.is_empty(), "{chat}");
        assert!(world.mark(chat).is_none(), "{chat}: an opt-out drops the mark");
        world.opt_out(chat, false)?;
        world.turn(chat, LARGE)?;
        world.fire();
        assert_eq!(world.compacts(chat).len(), 1, "{chat}: the chat fires after the opt-out goes");
    }
    Ok(())
}

#[test]
fn a_switch_turned_off_before_the_reservation_wins() -> TestResult {
    for chat in FIRING_CHATS {
        // Through the save: the switch in memory goes off.
        let (world, _) = marked(chat)?;
        world.switch(false)?;
        world.fire();
        assert!(world.compacts(chat).is_empty() && world.mark(chat).is_none(), "{chat}: save");
        // Through the row only: the sweep reads the row again after its
        // reservation.
        let (world, _) = marked(chat)?;
        let changed = world.sql("UPDATE configuration_entries SET payload='{\"enabled\":false}' WHERE kind='early_compaction_settings' AND id='global'", []);
        assert_eq!(changed?, 1);
        assert!(world.core.early_compaction.enabled(), "{chat}: memory still says on");
        world.fire();
        assert!(world.compacts(chat).is_empty() && world.attempts(chat)?.is_empty(), "{chat}: row");
        assert!(world.mark(chat).is_none(), "{chat}: disabled drops the mark");
    }
    Ok(())
}

#[test]
fn a_save_waits_for_a_dispatch_in_flight() -> TestResult {
    for chat in FIRING_CHATS {
        let (world, _) = marked(chat)?;
        let race = Race::arm(&world, |core| {
            let params = bridge_protocol::messages::SaveEarlyCompactionSettingsParams { session_id: None, enabled: Some(false) };
            crate::early_compaction_settings::save(&core, &params).is_ok()
        });
        world.fire();
        let (waited, saved) = race.finish()?;
        assert!(waited, "{chat}: the save did not wait for the dispatch");
        assert!(saved && !world.core.early_compaction.enabled(), "{chat}: the save finished after it");
        assert_eq!(world.compacts(chat).len(), 1, "{chat}");
    }
    Ok(())
}

#[test]
fn a_taken_lifecycle_claim_keeps_the_mark() -> TestResult {
    for chat in FIRING_CHATS {
        let (world, _) = marked(chat)?;
        let claim = world.core.claim_session_lifecycle(chat, "a test")?;
        world.fire();
        assert!(world.compacts(chat).is_empty(), "{chat}");
        assert!(world.mark(chat).is_some(), "{chat}: a taken claim keeps the mark");
        drop(claim);
        world.fire();
        assert_eq!(world.compacts(chat).len(), 1, "{chat}");
    }
    Ok(())
}

#[test]
fn idle_release_and_the_early_trigger_cannot_both_act() -> TestResult {
    // The release first: it stops the oldest idle runtime, the Claude chat,
    // and the sweep then finds no runtime to compact.
    let (world, _) = marked(CLAUDE_CHAT)?;
    crate::runtime_budget::IdleRuntimes::default().maintain(&world.core);
    assert!(!lock(&world.core.adapters).contains_key(CLAUDE_CHAT), "the release kept the Claude runtime");
    world.fire();
    assert!(world.compacts(CLAUDE_CHAT).is_empty() && world.attempts(CLAUDE_CHAT)?.is_empty());
    assert!(lock(&world.levers.launches).is_empty(), "the sweep started a process: {:?}", lock(&world.levers.launches));
    // The sweep first: while it dispatches it holds the lease alone, so the
    // release does nothing.
    let (world, _) = marked(CLAUDE_CHAT)?;
    let exclusive = Arc::new(Mutex::new(None));
    let (seen, core) = (exclusive.clone(), Arc::downgrade(&world.core));
    world.levers.on_compact(move || {
        if let Some(core) = core.upgrade() {
            *lock(&seen) = Some(core.input_activity.try_read().is_err());
            crate::runtime_budget::IdleRuntimes::default().maintain(&core);
        }
    });
    world.fire();
    assert_eq!(*lock(&exclusive), Some(true), "the dispatch ran without the lease");
    assert_eq!(world.compacts(CLAUDE_CHAT).len(), 1);
    assert_eq!(lock(&world.core.adapters).len(), 3, "the release acted during the dispatch");
    assert!(lock(&world.levers.launches).is_empty(), "a process started: {:?}", lock(&world.levers.launches));
    Ok(())
}

#[test]
fn an_early_compaction_never_starts_a_bridge_checkpoint() -> TestResult {
    let checkpoint = "SELECT COUNT(*) FROM session_entries WHERE session_id=?1 AND kind IN ('compaction','compaction.failed','checkpoint','compaction.requested')";
    for chat in FIRING_CHATS {
        let (world, _) = marked(chat)?;
        let during = Arc::new(Mutex::new(None));
        let (seen, core) = (during.clone(), Arc::downgrade(&world.core));
        world.levers.on_compact(move || {
            if let Some(core) = core.upgrade() {
                *lock(&seen) = status_in(&core, chat).ok();
            }
        });
        world.fire();
        assert_eq!(world.compacts(chat).len(), 1, "{chat}");
        assert_eq!(lock(&during).as_deref(), Some("ready"), "{chat}");
        assert_eq!(world.status(chat)?, "working", "{chat}");
        world.compaction(chat)?;
        assert_eq!(world.status(chat)?, "ready", "{chat}");
        assert_eq!(world.count(checkpoint, params![chat])?, 0, "{chat}");
        let pending = compaction_controller::CompactionController::pending(&lock(&world.core.db), chat)?;
        assert!(pending.is_none(), "{chat}");
    }
    Ok(())
}

#[test]
fn an_early_dispatch_that_finds_no_support_writes_no_checkpoint() -> TestResult {
    let checkpoint = "SELECT COUNT(*) FROM session_entries WHERE session_id=?1 AND kind IN ('compaction','compaction.failed','checkpoint','compaction.requested')";
    for chat in FIRING_CHATS {
        let (world, _) = marked(chat)?;
        world.take_calls();
        // The decision finds support. The dispatch asks again and finds none.
        world.levers.answer_support([None, Some(crate::adapters::NativeCompaction::Unsupported)]);
        world.fire();
        assert_eq!(world.attempt(chat)?.outcome.as_deref(), Some("unsupported"), "{chat}");
        assert_eq!(world.count(checkpoint, params![chat])?, 0, "{chat}: a checkpoint entry");
        let pending = compaction_controller::CompactionController::pending(&lock(&world.core.db), chat)?;
        assert!(pending.is_none(), "{chat}: a pending checkpoint");
        let calls = world.take_calls();
        assert!(calls.is_empty(), "{chat}: a turn went to the runtime: {calls:?}");
        assert_eq!(world.status(chat)?, "ready", "{chat}: the status changed");
    }
    Ok(())
}

#[test]
fn stop_during_an_early_compaction_settles_the_attempt_as_interrupted() -> TestResult {
    for chat in FIRING_CHATS {
        let world = fired(chat)?;
        stop_session(&world.core, chat.to_owned())?;
        world.sweep_after(WARM_SECONDS + 1);
        let attempt = world.attempt(chat)?;
        assert_eq!(attempt.outcome.as_deref(), Some("interrupted"), "{chat}");
        assert!(attempt.settled_at.is_some(), "{chat}");
        world.sweep_after(WARM_SECONDS + 2);
        assert_eq!(world.compacts(chat).len(), 1, "{chat}: a second send");
    }
    Ok(())
}

#[test]
fn an_early_fire_on_claude_sends_a_bare_compact() -> TestResult {
    let world = fired(CLAUDE_CHAT)?;
    assert_eq!(world.compacts(CLAUDE_CHAT), vec![json!([null])]);
    Ok(())
}

#[test]
fn an_early_fire_on_codex_sends_thread_compact_start() -> TestResult {
    let world = fired(CODEX_CHAT)?;
    assert_eq!(world.compacts(CODEX_CHAT), vec![json!([null])]);
    Ok(())
}

#[test]
fn an_opencode_chat_is_never_fired() -> TestResult {
    let world = World::new()?;
    world.switch(true)?;
    world.turn(OPENCODE_CHAT, LARGE)?;
    assert!(world.mark(OPENCODE_CHAT).is_none(), "an OpenCode turn gave a due mark");
    world.inject_mark(OPENCODE_CHAT, "opencode")?;
    world.fire();
    assert!(world.compacts(OPENCODE_CHAT).is_empty() && world.attempts(OPENCODE_CHAT)?.is_empty());
    assert!(world.mark(OPENCODE_CHAT).is_none(), "harness_not_allowed drops the mark");
    Ok(())
}

#[test]
fn an_attempt_row_exists_before_the_adapter_call() -> TestResult {
    for chat in FIRING_CHATS {
        let (world, _) = marked(chat)?;
        let seen = Arc::new(Mutex::new(None));
        let (slot, core) = (seen.clone(), Arc::downgrade(&world.core));
        world.levers.on_compact(move || {
            if let Some(core) = core.upgrade() {
                *lock(&slot) = Some(attempts_in(&core, chat));
            }
        });
        world.fire();
        let during = lock(&seen).take().ok_or("the adapter call did not run")??;
        assert_eq!(during.len(), 1, "{chat}: {during:?}");
        assert!(!during[0].dispatched && during[0].outcome.is_none(), "{chat}: {during:?}");
        let after = world.attempt(chat)?;
        assert_eq!(after.id, during[0].id, "{chat}");
        assert!(after.dispatched && after.outcome.is_none(), "{chat}: {after:?}");
    }
    Ok(())
}

#[test]
fn a_claude_boundary_settles_the_attempt_with_pre_and_post_tokens() -> TestResult {
    let (world, _) = marked(CLAUDE_CHAT)?;
    let mark = world.mark(CLAUDE_CHAT).ok_or("no due mark")?;
    let (trigger_id, _, trigger_used) = world.last_reading(CLAUDE_CHAT)?;
    world.fire();
    world.feed(CLAUDE_CHAT, &claude_compaction_with(Some(POST_TOKENS), SMALL))?;
    let attempt = world.attempt(CLAUDE_CHAT)?;
    assert_eq!(attempt.outcome.as_deref(), Some("compacted"));
    assert_eq!((attempt.pre, attempt.post), (Some(PRE_TOKENS), Some(POST_TOKENS)));
    assert_eq!(attempt.compaction_turn_id, None, "Claude has no turn id at the boundary");
    assert_eq!(attempt.trigger_turn_id, mark.turn_id);
    assert_eq!((attempt.trigger_reading_id, attempt.trigger_used_tokens), (trigger_id, trigger_used));
    assert_eq!(trigger_used, i64::try_from(LARGE)?);
    assert_eq!(attempt.reason, "window_over_threshold");
    assert_eq!(attempt.launch_started_at, world.launch(CLAUDE_CHAT)?.0);
    assert!(attempt.dispatched && attempt.settled_at.is_some());
    assert_eq!(world.status(CLAUDE_CHAT)?, "ready");
    Ok(())
}

#[test]
fn a_claude_boundary_without_post_tokens_settles_with_a_null_after_figure() -> TestResult {
    let world = fired(CLAUDE_CHAT)?;
    world.feed(CLAUDE_CHAT, &claude_compaction_with(None, SMALL))?;
    let attempt = world.attempt(CLAUDE_CHAT)?;
    assert_eq!(attempt.outcome.as_deref(), Some("compacted"));
    assert_eq!((attempt.pre, attempt.post), (Some(PRE_TOKENS), None));
    // The window after comes from the next reading, not from the boundary.
    world.sweep_after(WARM_SECONDS + 1);
    assert_eq!(world.attempt(CLAUDE_CHAT)?.next_used_tokens, Some(i64::try_from(SMALL)?));
    Ok(())
}

#[test]
fn a_codex_boundary_settles_with_absent_figures_and_labeled_readings() -> TestResult {
    let (world, turn) = marked(CODEX_CHAT)?;
    let (trigger_id, trigger_turn, _) = world.last_reading(CODEX_CHAT)?;
    assert_eq!(trigger_turn.as_deref(), Some(turn.as_str()), "the trigger reading has the turn id");
    world.fire();
    let compaction = world.compaction(CODEX_CHAT)?;
    let settled = world.attempt(CODEX_CHAT)?;
    assert_eq!(settled.outcome.as_deref(), Some("compacted"));
    assert_eq!((settled.pre, settled.post), (None, None), "Codex reports no figures");
    assert_eq!(settled.compaction_turn_id.as_deref(), Some(compaction.as_str()));
    assert_eq!(settled.trigger_turn_id.as_deref(), Some(turn.as_str()));
    assert_eq!(settled.trigger_reading_id, trigger_id);
    let next_turn = world.turn(CODEX_CHAT, SMALL)?;
    world.sweep_after(WARM_SECONDS + 1);
    let (next_id, next_reading_turn, next_used) = world.last_reading(CODEX_CHAT)?;
    assert_eq!(next_reading_turn.as_deref(), Some(next_turn.as_str()), "the next reading has the turn id");
    let filled = world.attempt(CODEX_CHAT)?;
    assert_eq!((filled.next_reading_id, filled.next_used_tokens), (Some(next_id), Some(next_used)));
    Ok(())
}

#[test]
fn a_boundary_before_the_dispatched_write_settles_its_attempt() -> TestResult {
    for chat in FIRING_CHATS {
        let (world, _) = marked(chat)?;
        let feeder = world.feeder(chat)?;
        let boundary = if chat == CLAUDE_CHAT {
            claude_boundary(Some(POST_TOKENS))
        } else {
            codex_boundary("codex-turn-early", "codex-compaction-early")
        };
        let seen = Arc::new(Mutex::new(None));
        let (slot, core) = (seen.clone(), Arc::downgrade(&world.core));
        world.levers.on_compact(move || {
            feeder.feed(&[boundary]);
            if let Some(core) = core.upgrade() {
                *lock(&slot) = Some(attempts_in(&core, chat));
            }
        });
        world.fire();
        let during = lock(&seen).take().ok_or("the adapter call did not run")??;
        let states: Vec<_> = during.iter().map(|attempt| (attempt.outcome.clone(), attempt.dispatched)).collect();
        assert_eq!(states, vec![(Some("compacted".to_owned()), false)], "{chat}");
        let after = world.attempt(chat)?;
        assert_eq!((after.outcome.as_deref(), after.dispatched), (Some("compacted"), true), "{chat}");
    }
    Ok(())
}

#[test]
fn a_later_manual_boundary_is_not_credited_to_an_early_attempt() -> TestResult {
    for chat in FIRING_CHATS {
        let world = fired(chat)?;
        world.turn_end_without_boundary(chat)?;
        let settled = world.attempt(chat)?;
        assert_eq!(settled.outcome.as_deref(), Some("ended_without_boundary"), "{chat}");
        world.manual_compaction(chat)?;
        world.sweep_after(WARM_SECONDS + 1);
        assert_eq!(world.compacts(chat).len(), 2, "{chat}: the manual compaction did not run");
        assert_eq!(world.attempt(chat)?, settled, "{chat}: the manual boundary changed the early row");
    }
    Ok(())
}

#[test]
fn a_boundary_of_another_launch_settles_nothing() -> TestResult {
    for chat in FIRING_CHATS {
        let world = fired(chat)?;
        let frames = if chat == CLAUDE_CHAT {
            claude_compaction_with(Some(POST_TOKENS), SMALL)
        } else {
            codex_compaction("codex-turn-old", "codex-compaction-old")
        };
        // A reader of an older launch.
        world.feed_as(chat, ("2026-01-01T00:00:00+00:00", "an-older-thread"), &frames)?;
        assert_eq!(world.attempt(chat)?.outcome, None, "{chat}: older launch");
        // The reader of a newer launch.
        world.relaunch(chat)?;
        world.compaction(chat)?;
        assert_eq!(world.attempt(chat)?.outcome, None, "{chat}: newer launch");
        world.sweep_after(WARM_SECONDS + 1);
        assert_eq!(world.attempt(chat)?.outcome.as_deref(), Some("interrupted"), "{chat}");
    }
    Ok(())
}

#[test]
fn a_turn_end_without_a_boundary_settles_as_ended_without_boundary() -> TestResult {
    for chat in FIRING_CHATS {
        let world = fired(chat)?;
        world.turn_end_without_boundary(chat)?;
        let attempt = world.attempt(chat)?;
        assert_eq!(attempt.outcome.as_deref(), Some("ended_without_boundary"), "{chat}");
        assert!(attempt.settled_at.is_some() && attempt.pre.is_none() && attempt.compaction_turn_id.is_none(), "{chat}");
        assert!(world.mark(chat).is_none(), "{chat}: the turn of the attempt gave a due mark");
        // This outcome does not stop the launch.
        world.turn(chat, LARGE)?;
        world.fire();
        assert_eq!(world.attempts(chat)?.len(), 2, "{chat}");
    }
    Ok(())
}

#[test]
fn an_adapter_error_settles_as_failed_and_leaves_the_status() -> TestResult {
    for chat in FIRING_CHATS {
        let (world, _) = marked(chat)?;
        world.levers.fail_compact("the harness refused the request");
        world.fire();
        let attempt = world.attempt(chat)?;
        assert_eq!(attempt.outcome.as_deref(), Some("failed"), "{chat}");
        assert!(attempt.error.as_deref().is_some_and(|error| error.contains("the harness refused the request")), "{chat}: {attempt:?}");
        assert!(!attempt.dispatched && attempt.settled_at.is_some(), "{chat}");
        assert_eq!(world.status(chat)?, "ready", "{chat}");
        // A failed attempt stops the launch.
        world.turn(chat, LARGE)?;
        world.fire();
        assert_eq!(world.attempts(chat)?.len(), 1, "{chat}");
    }
    Ok(())
}

#[test]
fn a_failed_compaction_turn_keeps_the_provider_error() -> TestResult {
    let long = "The request failed. ".repeat(40);
    for chat in FIRING_CHATS {
        for error in ["The model is overloaded.", long.as_str()] {
            let world = fired(chat)?;
            world.failed_turn_end(chat, error)?;
            let attempt = world.attempt(chat)?;
            assert_eq!(attempt.outcome.as_deref(), Some("failed"), "{chat}");
            // The column keeps the first 500 characters.
            let kept: String = error.chars().take(500).collect();
            assert_eq!(attempt.error.as_deref(), Some(kept.as_str()), "{chat}: the provider error is lost");
        }
    }
    Ok(())
}

#[test]
fn no_terminal_signal_within_the_limit_settles_as_unsettled() -> TestResult {
    // The attempt starts at `WARM_SECONDS` and the limit is 300 seconds.
    for chat in FIRING_CHATS {
        let world = fired(chat)?;
        world.sweep_after(320);
        assert_eq!(world.attempt(chat)?.outcome, None, "{chat}: within the limit");
        world.sweep_after(340);
        let attempt = world.attempt(chat)?;
        assert_eq!(attempt.outcome.as_deref(), Some("unsettled"), "{chat}");
        assert!(attempt.settled_at.is_some(), "{chat}");
        assert_eq!(world.status(chat)?, "working", "{chat}: unsettled changed the status");
    }
    Ok(())
}

#[test]
fn a_launch_change_settles_the_open_attempt_as_interrupted() -> TestResult {
    let changes = [
        "UPDATE sessions SET started_at='2026-11-20T00:00:00+00:00' WHERE id=?1",
        "UPDATE sessions SET provider_session_id='another-thread' WHERE id=?1",
    ];
    for chat in FIRING_CHATS {
        for change in changes {
            let world = fired(chat)?;
            world.sweep_after(WARM_SECONDS + 1);
            assert_eq!(world.attempt(chat)?.outcome, None, "{chat}: the current launch keeps it open");
            world.sql(change, params![chat])?;
            world.sweep_after(WARM_SECONDS + 2);
            let attempt = world.attempt(chat)?;
            assert_eq!(attempt.outcome.as_deref(), Some("interrupted"), "{chat}: {change}");
            assert!(attempt.settled_at.is_some(), "{chat}");
        }
    }
    Ok(())
}

#[test]
fn an_unsettled_attempt_stops_further_attempts_on_that_launch() -> TestResult {
    for chat in FIRING_CHATS {
        let world = fired(chat)?;
        world.sweep_after(340);
        assert_eq!(world.attempt(chat)?.outcome.as_deref(), Some("unsettled"), "{chat}");
        world.idle(chat)?;
        world.turn(chat, LARGE)?;
        world.fire();
        assert_eq!(world.attempts(chat)?.len(), 1, "{chat}: a second attempt on the same launch");
        // A new launch can try again.
        world.relaunch(chat)?;
        world.turn(chat, LARGE)?;
        world.fire();
        assert_eq!(world.attempts(chat)?.len(), 2, "{chat}: the new launch");
    }
    Ok(())
}

#[test]
fn the_next_reading_fills_after_a_compacted_attempt() -> TestResult {
    for chat in FIRING_CHATS {
        let world = fired(chat)?;
        world.compaction(chat)?;
        assert_eq!(world.attempt(chat)?.next_reading_id, None, "{chat}: the fill waits for a sweep");
        if chat == CODEX_CHAT {
            world.turn(chat, SMALL)?;
        }
        world.sweep_after(WARM_SECONDS + 1);
        let (id, _, used) = world.last_reading(chat)?;
        let filled = world.attempt(chat)?;
        assert_eq!((filled.next_reading_id, filled.next_used_tokens), (Some(id), Some(used)), "{chat}");
        assert_eq!(used, i64::try_from(SMALL)?, "{chat}");
        assert!(id > filled.trigger_reading_id, "{chat}");
        // A later reading does not replace it.
        world.turn(chat, 50_000)?;
        world.sweep_after(WARM_SECONDS + 2);
        assert_eq!(world.attempt(chat)?.next_reading_id, Some(id), "{chat}");
    }
    Ok(())
}

#[test]
fn a_claude_reading_after_the_compaction_result_is_the_next_reading() -> TestResult {
    let world = fired(CLAUDE_CHAT)?;
    world.feed(CLAUDE_CHAT, &claude_compaction_start())?;
    world.feed(CLAUDE_CHAT, &[claude_reading(140_000)])?;
    let (before_id, _, _) = world.last_reading(CLAUDE_CHAT)?;
    world.feed(CLAUDE_CHAT, &[claude_boundary(Some(POST_TOKENS))])?;
    world.sweep_after(WARM_SECONDS + 1);
    let settled = world.attempt(CLAUDE_CHAT)?;
    assert!(before_id > settled.trigger_reading_id, "the reading before the boundary is newer than the trigger");
    assert_eq!(settled.next_reading_id, None, "a reading from before the boundary is not the window after");
    world.feed(CLAUDE_CHAT, &claude_compaction_end(SMALL))?;
    world.sweep_after(WARM_SECONDS + 2);
    let (after_id, _, _) = world.last_reading(CLAUDE_CHAT)?;
    let filled = world.attempt(CLAUDE_CHAT)?;
    assert_eq!((filled.next_reading_id, filled.next_used_tokens), (Some(after_id), Some(i64::try_from(SMALL)?)));
    Ok(())
}

#[test]
fn a_codex_reading_of_the_compaction_turn_is_not_the_next_reading() -> TestResult {
    let world = fired(CODEX_CHAT)?;
    let (turn, item) = ("codex-turn-90", "codex-compaction-90");
    let mut frames = codex_compaction_start(3, turn, item);
    frames.push(codex_reading(turn, 40_000));
    frames.push(codex_boundary(turn, item));
    frames.push(codex_reading(turn, 32_942));
    frames.push(codex_turn_completed(turn, "completed"));
    world.feed(CODEX_CHAT, &frames)?;
    let settled = world.attempt(CODEX_CHAT)?;
    assert_eq!(settled.compaction_turn_id.as_deref(), Some(turn));
    let labeled = world.readings(CODEX_CHAT)?.into_iter().filter(|(_, reading_turn, _)| reading_turn.as_deref() == Some(turn)).count();
    assert_eq!(labeled, 2, "the readings of the compaction turn carry its turn id");
    // Also with a time equal to the time of the boundary.
    let moved = world.sql(
        "UPDATE context_readings SET created_at=(SELECT settled_at FROM early_compaction_attempts WHERE session_id=?1)
         WHERE session_id=?1 AND turn_id=?2",
        params![CODEX_CHAT, turn],
    )?;
    assert_eq!(moved, 2);
    world.sweep_after(WARM_SECONDS + 1);
    assert_eq!(world.attempt(CODEX_CHAT)?.next_reading_id, None);
    let next_turn = world.turn(CODEX_CHAT, SMALL)?;
    world.sweep_after(WARM_SECONDS + 2);
    let (id, reading_turn, _) = world.last_reading(CODEX_CHAT)?;
    assert_eq!(reading_turn.as_deref(), Some(next_turn.as_str()));
    assert_eq!(world.attempt(CODEX_CHAT)?.next_reading_id, Some(id));
    Ok(())
}

#[test]
fn a_reading_of_a_later_launch_does_not_fill_the_attempt() -> TestResult {
    for (chat, harness) in [(CLAUDE_CHAT, "claude"), (CODEX_CHAT, "codex")] {
        let world = fired(chat)?;
        // Launch A gets its boundary and stops before its next reading.
        if chat == CLAUDE_CHAT {
            let mut frames = claude_compaction_start();
            frames.push(claude_boundary(Some(POST_TOKENS)));
            world.feed(chat, &frames)?;
        } else {
            world.compaction(chat)?;
        }
        assert_eq!(world.attempt(chat)?.outcome.as_deref(), Some("compacted"), "{chat}");
        // Launch B resumes the same provider session with the same model.
        world.relaunch(chat)?;
        world.turn(chat, LARGE)?;
        world.sweep_after(1);
        assert_eq!(world.attempt(chat)?.next_reading_id, None, "{chat}: a reading of launch B filled the attempt of launch A");
        assert!(world.core.early_compaction.fills().is_empty(), "{chat}: the fill of launch A stays");
        world.inject_mark(chat, harness)?;
        let memory = world.core.early_compaction.memory_inputs(chat, Utc::now()).ok_or("no mark")?;
        assert_eq!(memory.launch_stop, None, "{chat}: launch B is stopped");
    }
    Ok(())
}

/// A whole compaction of `chat`. On Claude, the reading after the result
/// has `used` tokens. On Codex, the next turn gives the next reading.
fn compaction_then_reading(world: &World, chat: &str, used: u64) -> TestResult {
    if chat == CLAUDE_CHAT {
        world.feed(chat, &claude_compaction_with(Some(POST_TOKENS), used))
    } else {
        world.compaction(chat).map(|_| ())
    }
}

#[test]
fn a_next_reading_at_the_threshold_stops_further_attempts_on_that_launch() -> TestResult {
    let at = crate::early_compaction::THRESHOLD_TOKENS;
    for chat in FIRING_CHATS {
        let world = fired(chat)?;
        compaction_then_reading(&world, chat, u64::try_from(at)?)?;
        // This turn reads at the threshold too, so only the stop holds it.
        world.turn(chat, u64::try_from(at)?)?;
        world.fire();
        assert_eq!(world.attempt(chat)?.next_used_tokens, Some(at), "{chat}");
        world.turn(chat, LARGE)?;
        world.fire();
        assert_eq!(world.attempts(chat)?.len(), 1, "{chat}: a second attempt on the same launch");
        world.relaunch(chat)?;
        world.turn(chat, LARGE)?;
        world.fire();
        assert_eq!(world.attempts(chat)?.len(), 2, "{chat}: the new launch");
    }
    Ok(())
}

#[test]
fn a_next_reading_below_the_threshold_lets_a_later_turn_fire() -> TestResult {
    let below = crate::early_compaction::THRESHOLD_TOKENS - 1;
    for chat in FIRING_CHATS {
        let world = fired(chat)?;
        compaction_then_reading(&world, chat, u64::try_from(below)?)?;
        world.turn(chat, u64::try_from(below)?)?;
        world.fire();
        assert_eq!(world.attempt(chat)?.next_used_tokens, Some(below), "{chat}");
        world.turn(chat, LARGE)?;
        world.fire();
        assert_eq!(world.attempts(chat)?.len(), 2, "{chat}");
        assert_eq!(world.compacts(chat).len(), 2, "{chat}");
    }
    Ok(())
}

#[test]
fn the_turn_end_after_a_boundary_does_not_change_the_settled_row() -> TestResult {
    for chat in FIRING_CHATS {
        let world = fired(chat)?;
        let (turn, item) = ("codex-turn-91", "codex-compaction-91");
        let (start, end) = if chat == CLAUDE_CHAT {
            let mut start = claude_compaction_start();
            start.push(claude_boundary(Some(POST_TOKENS)));
            (start, claude_compaction_end(SMALL))
        } else {
            let mut start = codex_compaction_start(3, turn, item);
            start.push(codex_reading(turn, 32_942));
            start.push(codex_boundary(turn, item));
            (start, vec![codex_turn_completed(turn, "completed")])
        };
        world.feed(chat, &start)?;
        let settled = world.attempt(chat)?;
        assert_eq!(settled.outcome.as_deref(), Some("compacted"), "{chat}");
        world.feed(chat, &end)?;
        assert_eq!(world.attempt(chat)?, settled, "{chat}");
        assert!(world.mark(chat).is_none(), "{chat}");
    }
    Ok(())
}

#[test]
fn disable_does_not_skip_settlement() -> TestResult {
    for chat in FIRING_CHATS {
        // By a frame.
        let world = fired(chat)?;
        world.switch(false)?;
        world.compaction(chat)?;
        assert_eq!(world.attempt(chat)?.outcome.as_deref(), Some("compacted"), "{chat}: frame");
        // By the sweep.
        let world = fired(chat)?;
        world.switch(false)?;
        world.sweep_after(340);
        assert_eq!(world.attempt(chat)?.outcome.as_deref(), Some("unsettled"), "{chat}: sweep");
    }
    Ok(())
}

#[test]
fn an_open_attempt_is_settled_as_interrupted_at_startup_with_no_second_send() -> TestResult {
    for chat in FIRING_CHATS {
        let world = fired(chat)?;
        let restarted = world.restart()?;
        assert!(restarted.core.early_compaction.enabled(), "{chat}: the restart reads the switch");
        let mut seen = recover_before_maintenance(&restarted.core);
        let attempt = restarted.attempt(chat)?;
        assert_eq!(attempt.outcome.as_deref(), Some("interrupted"), "{chat}");
        assert!(attempt.settled_at.is_some(), "{chat}");
        restarted.fire();
        queued_input_maintenance_tick(&restarted.core, &mut seen);
        assert!(restarted.compacts(chat).is_empty(), "{chat}: a second send");
        assert_eq!(restarted.attempts(chat)?.len(), 1, "{chat}");
    }
    Ok(())
}

#[test]
fn with_the_switch_off_the_sweep_runs_no_sql() -> TestResult {
    for saved in [None, Some(false)] {
        let world = World::new()?;
        if let Some(enabled) = saved {
            world.switch(enabled)?;
        }
        world.inject_mark(CLAUDE_CHAT, "claude")?;
        let ((), traced) = statements(&world.core, || world.fire());
        assert!(traced.is_empty(), "{saved:?}: {traced:?}");
        assert!(world.mark(CLAUDE_CHAT).is_none(), "{saved:?}: the mark stayed");
    }
    // The trace sees the sweep when the switch is on.
    let world = World::new()?;
    world.switch(true)?;
    world.inject_mark(CLAUDE_CHAT, "claude")?;
    let ((), traced) = statements(&world.core, || world.fire());
    assert!(!traced.is_empty());
    Ok(())
}

#[test]
fn with_the_switch_off_the_attempt_table_stays_empty() -> TestResult {
    for saved in [None, Some(false)] {
        let world = World::new()?;
        if let Some(enabled) = saved {
            world.switch(enabled)?;
        }
        let mut seen = 0;
        for chat in FIRING_CHATS {
            world.turn(chat, LARGE)?;
            world.fire();
            queued_input_maintenance_tick(&world.core, &mut seen);
            world.manual_compaction(chat)?;
            world.fire();
            queued_input_maintenance_tick(&world.core, &mut seen);
            assert_eq!(world.compacts(chat), vec![json!([null])], "{saved:?} {chat}: only the manual compaction");
        }
        assert_eq!(world.count("SELECT COUNT(*) FROM early_compaction_attempts", [])?, 0, "{saved:?}");
    }
    Ok(())
}

#[test]
fn start_session_skips_an_aside_with_a_pending_fork() {
    let (dir, core, script) = live();
    let chat = workspace_chat(&core, dir.path(), "claude", "source-thread");
    let aside = "aside";
    {
        let db = core.db.lock().unwrap();
        // The aside keeps the thread of its source and a pending fork.
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,depth,kind,provider_session_id) VALUES(?1,'w','claude','Aside','stopped','reported',0,'direct','source-thread')",
            params![aside],
        )
        .unwrap();
        restoration::set_head_state(&db, aside, RestorationMode::NativeFork, crate::model::ResumeEligibility::Native, Some("source-thread")).unwrap();
        db.execute("UPDATE sessions SET harness='codex' WHERE id=?1", params![chat]).unwrap();
    }
    start_session(&core, "w".into(), Some(Harness::Claude), None).unwrap();
    let launched = asked(&script);
    assert_eq!(verbs(&launched), [("start", None)], "{:?}", verbs(&launched));
    assert!(warm_thread(&core, aside).is_none());
    let started = new_session_of(&core, "w", "claude", &[aside]);
    assert!(warm_thread(&core, &started).is_some());
}

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

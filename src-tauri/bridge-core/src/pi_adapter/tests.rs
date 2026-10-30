use super::*;
use std::io::BufRead;

#[cfg(unix)]
fn fake(mode: &str) -> (tempfile::TempDir, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pi");
    let python = crate::binary::resolve("python3").expect("fake Pi needs python3");
    let script = r#"
import sys,json,pathlib,time,os
MODE='MODE_VALUE'
args=sys.argv[1:]
if '--version' in args:
    print('0.60.0' if MODE=='old-version' else '1.1.0'); sys.exit(0)
def emit(value):
    sys.stdout.write(json.dumps(value,ensure_ascii=False)+'\n'); sys.stdout.flush()
def flag(name):
    return args[args.index(name)+1] if name in args else None
folder=pathlib.Path(flag('--session-dir'))
session=flag('--session') or str(folder/'fake.jsonl')
if '--no-session' not in args: pathlib.Path(session).write_text('{}\n')
log=pathlib.Path(__file__).with_name('commands.jsonl')
model={'provider':'local','id':'one','name':'One','headers':{'Authorization':'SECRET'},'apiKey':'SECRET','baseUrl':'http://localhost/private'}
other={'provider':'local','id':'nested/two','name':'Two'}
streaming=False
if MODE=='early-exit':
    sys.stderr.write('Authorization: SECRET\n');sys.stderr.flush();sys.exit(7)
if MODE!='no-gate': emit({'type':'extension_ui_request','method':'notify','message':'bridge-pi-permissions-v1','id':'ready'})
for line in sys.stdin.buffer:
    cmd=json.loads(line); typ=cmd['type']; rid=cmd.get('id')
    with log.open('a') as f: f.write(json.dumps(cmd)+'\n')
    if typ=='extension_ui_response':
        if cmd.get('cancelled'):
            approval_cancelled=True;continue
        emit({'type':'tool_execution_end','toolCallId':'tool','toolName':'read','result':{'content':[{'type':'text','text':'result'}]},'isError':not cmd.get('confirmed',False)})
        emit({'type':'message_end','message':{'role':'assistant','timestamp':10,'content':[{'type':'thinking','thinking':'Plan'},{'type':'text','text':'Reply\u2028line'}],'provider':'local','model':'one','usage':{'input':12,'output':8,'cacheRead':3,'cacheWrite':4,'cost':{'total':0.01}},'stopReason':'stop'}})
        emit({'type':'agent_end','willRetry':False}); emit({'type':'agent_settled','aborted':False}); streaming=False
        continue
    if typ=='stop-reading':
        emit({'type':'response','id':rid,'command':typ,'success':True});time.sleep(30);continue
    if typ=='hang': continue
    if typ=='malformed': sys.stdout.write('SECRET malformed\n');sys.stdout.flush(); continue
    if typ=='truncated': sys.stdout.write('{"type":"private"}');sys.stdout.flush();sys.exit(1)
    if typ=='exit': sys.stderr.write('SECRET diagnostics\n');sys.stderr.flush();sys.exit(7)
    if typ=='get_state': data={'sessionId':'uuid','sessionFile':session if '--no-session' not in args else None,'isStreaming':streaming,'model':model,'thinkingLevel':globals().get('level','medium')}
    elif typ=='get_available_models': data={'models':[model,other]}
    elif typ=='set_model': model={'provider':cmd['provider'],'id':cmd['modelId'],'name':'Selected'}; data=model
    elif typ=='get_available_thinking_levels': data={'levels':['off'] if model['id']=='nested/two' else ['off','minimal','low','medium','high']}
    elif typ=='set_thinking_level': level=cmd['level'];data=None
    elif typ=='prompt':
        if MODE=='handled': data={'disposition':'handled'}
        elif MODE=='reject-prompt': data=None
        elif streaming: data={'disposition':'queued'}
        else:
            streaming=True
            emit({'type':'agent_start'})
            emit({'type':'message_start','message':{'role':'assistant','timestamp':10}})
            emit({'type':'message_update','assistantMessageEvent':{'type':'thinking_delta','contentIndex':0,'delta':'Plan'}})
            emit({'type':'message_update','assistantMessageEvent':{'type':'thinking_end','contentIndex':0,'content':'Plan'}})
            emit({'type':'message_update','assistantMessageEvent':{'type':'text_delta','contentIndex':1,'delta':'Reply\u2028line'}})
            emit({'type':'extension_ui_request','method':'confirm','id':'permission','title':json.dumps({'bridgePermission':1,'toolCallId':'tool','toolName':'read'}),'message':'{"path":"file"}'})
            data={'disposition':'started'}
    elif typ=='abort':
        if not globals().get('approval_cancelled',False): time.sleep(15)
        emit({'type':'agent_settled','aborted':True});streaming=False;data=None
    elif typ=='compact': emit({'type':'compaction_start','reason':'manual'});emit({'type':'compaction_end','reason':'manual','result':{'tokensBefore':100,'estimatedTokensAfter':10},'willRetry':False});data={'summary':'Summary'}
    elif typ=='pressure':
        for i in range(2200): emit({'type':'message_update','assistantMessageEvent':{'type':'text_delta','contentIndex':0,'delta':'x'}})
        data={'done':True}
    elif typ=='reverse':
        previous=globals().get('previous')
        if not previous: previous=cmd;continue
        emit({'type':'response','id':rid,'command':typ,'success':True,'data':{'order':2}})
        emit({'type':'response','id':previous['id'],'command':typ,'success':True,'data':{'order':1}})
        previous=None;continue
    else: data=None
    emit({'type':'queue_update','steering':[],'followUp':[]})
    emit({'type':'response','id':rid,'command':'WRONG' if MODE=='wrong-command' else typ,'success':typ!='reject' and not (MODE=='reject-prompt' and typ=='prompt'),'data':data,'error':'SECRET'})
    if MODE=='duplicates': emit({'type':'response','id':rid,'command':typ,'success':True,'data':{'wrong':'duplicate'}})
"#;
    std::fs::write(
        &path,
        format!(
            "#!{}\n{}",
            python.display(),
            script.replace("MODE_VALUE", mode)
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    (directory, path)
}
fn request(cwd: &str) -> StartRequest<'_> {
    StartRequest {
        cwd,
        model: None,
        effort: None,
        instructions: None,
        write_mode: None,
        read_only_sandbox: None,
        briefing: None,
        interactive: false,
        account_home: None,
        on_progress: None,
    }
}
fn read_until(reader: &mut impl BufRead, kind: &str) -> Vec<Value> {
    let mut output = Vec::new();
    loop {
        let mut line = String::new();
        assert!(
            reader.read_line(&mut line).unwrap() > 0,
            "EOF before {kind}"
        );
        let value: Value = serde_json::from_str(&line).unwrap();
        let done = value["event"]["type"] == kind;
        output.push(value);
        if done {
            return output;
        }
    }
}
#[cfg(unix)]
#[test]
fn fake_pi_discovery_projects_models_without_credentials() {
    let (_directory, path) = fake("normal");
    let profile = PiAdapter::discover(&path).unwrap();
    assert_eq!(profile.version, "1.1.0");
    assert_eq!(profile.default_model.as_deref(), Some("local/one"));
    let encoded = serde_json::to_string(&profile.models).unwrap();
    for forbidden in ["SECRET", "Authorization", "private", "baseUrl", "apiKey"] {
        assert!(!encoded.contains(forbidden));
    }
    assert_eq!(
        profile
            .models
            .iter()
            .find(|model| model.id == "local/nested/two")
            .unwrap()
            .supported_effort_levels,
        ["off"]
    );
    assert_eq!(
        profile
            .models
            .iter()
            .find(|model| model.id == "local/one")
            .unwrap()
            .supported_effort_levels,
        ["off", "minimal", "low", "medium", "high"]
    );
}
#[cfg(unix)]
#[test]
fn fake_pi_user_journey_streams_permissions_settlement_interrupt_resume_and_shutdown() {
    let (directory, path) = fake("normal");
    let cwd = directory.path().to_str().unwrap();
    let (mut runtime, mut reader) =
        PiRuntime::open(&path, directory.path(), request(cwd), None, false).unwrap();
    let pid = runtime.process_id();
    let saved = runtime.provider_session_id().to_owned();
    let levels = select_model(&runtime.process.client, "local/one").unwrap();
    assert!(levels.contains(&"medium".into()));
    runtime
        .send_turn_with_context(
            "Review",
            TurnContext {
                session: Some("PRIVATE CONTEXT"),
                credentials: None,
            },
        )
        .unwrap();
    let mut events = read_until(&mut reader, "extension_ui_request");
    assert!(runtime.respond(json!("made-up"), "accept").is_err());
    runtime.respond(json!("permission"), "accept").unwrap();
    assert!(runtime.respond(json!("permission"), "accept").is_err());
    events.extend(read_until(&mut reader, "agent_settled"));
    let mut state = normalize::StreamState::default();
    let normalized = events
        .iter()
        .flat_map(|value| state.normalize(&value["event"]))
        .collect::<Vec<_>>();
    for kind in [
        "turn.started",
        "reasoning.completed",
        "permission.requested",
        "tool.completed",
        "message.completed",
        "usage.updated",
        "turn.completed",
    ] {
        assert!(
            normalized.iter().any(|event| event.kind == kind),
            "missing {kind}"
        );
    }
    runtime.send_turn("Next").unwrap();
    runtime.send_turn("Steer").unwrap();
    runtime.interrupt().unwrap();
    let interrupted = read_until(&mut reader, "agent_settled");
    assert_eq!(interrupted.last().unwrap()["event"]["aborted"], true);
    runtime.compact_native(Some("Keep decisions")).unwrap();
    // Manual compaction runs outside an agent run, so its end is the boundary
    // that returns the Bridge session to ready.
    let mut compaction = normalize::StreamState::default();
    let compacted = read_until(&mut reader, "compaction_end")
        .iter()
        .flat_map(|value| compaction.normalize(&value["event"]))
        .collect::<Vec<_>>();
    assert!(compacted.iter().any(|event| event.kind == "context.compacted"));
    assert_eq!(compacted.last().unwrap().kind, "turn.completed");
    assert_eq!(compacted.last().unwrap().status.as_deref(), Some("completed"));
    let response = read_until(&mut reader, "bridge_compaction_response");
    assert!(compaction.normalize(&response.last().unwrap()["event"]).is_empty());
    assert!(runtime
        .send_turn_with_context(
            "Key",
            TurnContext {
                session: None,
                credentials: Some("SECRET")
            }
        )
        .is_err());
    assert!(runtime.send_turn("/reload").is_err());
    assert!(runtime.send_turn("!bash").is_err());
    runtime.stop(ShutdownReason::Replaced);
    assert!(runtime.process.child.try_wait().unwrap().is_some());
    assert!(!crate::adapters::process_identity(pid).is_some());
    let (mut resumed, _reader) =
        PiRuntime::open(&path, directory.path(), request(cwd), Some(&saved), false).unwrap();
    assert_eq!(resumed.provider_session_id(), saved);
    resumed.stop(ShutdownReason::AppShutdown);
    let log = std::fs::read_to_string(directory.path().join("commands.jsonl")).unwrap();
    assert!(!log.contains("PRIVATE CONTEXT"));
    assert!(!log.contains("SECRET"));
    let commands: Vec<Value> = log
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let abort = commands
        .iter()
        .position(|cmd| cmd["type"] == "abort")
        .unwrap();
    let clear = commands[..abort]
        .iter()
        .rposition(|cmd| cmd["type"] == "clear_queue")
        .unwrap();
    assert!(commands[clear + 1..abort]
        .iter()
        .any(|cmd| cmd["type"] == "extension_ui_response" && cmd["cancelled"] == true));
    assert!(commands
        .iter()
        .any(|cmd| cmd["type"] == "compact" && cmd["customInstructions"] == "Keep decisions"));
    assert!(commands
        .iter()
        .any(|cmd| cmd["message"] == "Steer" && cmd["streamingBehavior"] == "steer"));
}
#[cfg(unix)]
#[test]
fn fake_pi_correlates_concurrent_out_of_order_and_duplicate_responses() {
    let (directory, path) = fake("duplicates");
    let cwd = directory.path().to_str().unwrap();
    let (runtime, _reader) =
        PiRuntime::open(&path, directory.path(), request(cwd), None, true).unwrap();
    let client = runtime.process.client.clone();
    let second = client.clone();
    let a = thread::spawn(move || {
        client
            .command(json!({"type":"reverse"}), Duration::from_secs(2))
            .unwrap()
    });
    let b = thread::spawn(move || {
        second
            .command(json!({"type":"reverse"}), Duration::from_secs(2))
            .unwrap()
    });
    let mut orders = vec![
        a.join().unwrap()["order"].as_u64().unwrap(),
        b.join().unwrap()["order"].as_u64().unwrap(),
    ];
    orders.sort();
    assert_eq!(orders, [1, 2]);
    assert!(runtime
        .process
        .client
        .command(json!({"type":"reject"}), Duration::from_secs(1))
        .unwrap_err()
        .to_string()
        .contains("rejected"));
    assert!(runtime
        .process
        .client
        .command(json!({"type":"hang"}), Duration::from_millis(50))
        .is_err());
    assert!(runtime
        .process
        .client
        .command(json!({"type":"get_state"}), Duration::from_secs(1))
        .unwrap()
        .get("sessionId")
        .is_some());
}
#[cfg(unix)]
#[test]
fn fake_pi_bounds_stdout_pressure_without_losing_command_responses() {
    let (directory, path) = fake("normal");
    let cwd = directory.path().to_str().unwrap();
    let (runtime, _reader) =
        PiRuntime::open(&path, directory.path(), request(cwd), None, true).unwrap();
    assert_eq!(
        runtime
            .process
            .client
            .command(json!({"type":"pressure"}), Duration::from_secs(5))
            .unwrap()["done"],
        true
    );
    let metrics = runtime.event_queue_metrics().unwrap();
    assert!(metrics.dropped_transient > 0);
    assert!(metrics.depth <= 2048);
    assert!(metrics.bytes <= 8 * 1024 * 1024);
}
#[cfg(unix)]
#[test]
fn fake_pi_refuses_unproven_gates_wrong_versions_and_incompatible_replies() {
    for mode in ["no-gate", "wrong-command", "early-exit"] {
        let (directory, path) = fake(mode);
        let cwd = directory.path().to_str().unwrap();
        let error = PiRuntime::open(&path, directory.path(), request(cwd), None, true)
            .err()
            .expect("must refuse")
            .to_string();
        assert!(!error.contains("SECRET"));
        if mode == "no-gate" {
            assert!(error.contains("permission extension"));
        }
    }
    let (directory, path) = fake("old-version");
    assert!(version(&path, directory.path().to_str().unwrap())
        .unwrap_err()
        .to_string()
        .contains("Pi 1.1+"));
}
#[cfg(unix)]
#[test]
fn fake_pi_protocol_failure_and_exit_are_actionable_without_diagnostics() {
    for command in ["malformed", "truncated", "exit"] {
        let (directory, path) = fake("normal");
        let cwd = directory.path().to_str().unwrap();
        let (mut runtime, mut reader) =
            PiRuntime::open(&path, directory.path(), request(cwd), None, true).unwrap();
        let result = runtime
            .process
            .client
            .command(json!({"type":command}), Duration::from_secs(3));
        assert!(result.is_err());
        assert!(!result.unwrap_err().to_string().contains("SECRET"));
        let events = read_until(&mut reader, "bridge_error");
        assert!(!format!("{events:?}").contains("SECRET"));
        runtime.stop(ShutdownReason::Failed);
        assert!(runtime.process.child.try_wait().unwrap().is_some());
        assert!(!runtime.failure_context().unwrap().contains("SECRET"));
    }
}
#[cfg(unix)]
#[test]
fn handled_prompts_finish_and_drop_reaps_the_child() {
    let (directory, path) = fake("handled");
    let cwd = directory.path().to_str().unwrap();
    let (runtime, mut reader) =
        PiRuntime::open(&path, directory.path(), request(cwd), None, true).unwrap();
    let pid = runtime.process_id();
    runtime.send_turn("Handled").unwrap();
    let events = read_until(&mut reader, "bridge_handled");
    assert_eq!(
        normalize::StreamState::default().normalize(&events.last().unwrap()["event"])[0].kind,
        "turn.completed"
    );
    drop(runtime);
    assert!(crate::adapters::process_identity(pid).is_none());
}
#[cfg(unix)]
#[test]
fn rejected_prompts_settle_the_turn_instead_of_blocking_the_sender() {
    let (directory, path) = fake("reject-prompt");
    let cwd = directory.path().to_str().unwrap();
    let (runtime, mut reader) =
        PiRuntime::open(&path, directory.path(), request(cwd), None, true).unwrap();
    runtime.send_turn("Hello").unwrap();
    let events = read_until(&mut reader, "bridge_prompt_rejected");
    let settled = normalize::StreamState::default().normalize(&events.last().unwrap()["event"]);
    assert_eq!(
        settled
            .iter()
            .map(|event| (event.kind.as_str(), event.status.as_deref()))
            .collect::<Vec<_>>(),
        [("turn.completed", Some("failed")), ("error", Some("failed"))]
    );
    assert!(!format!("{settled:?}").contains("SECRET"));
}
#[test]
fn oversized_catalogs_keep_the_default_model_instead_of_failing() {
    let models = (0..400)
        .map(|index| json!({"provider":"openrouter","id":format!("vendor/model-{index}")}))
        .collect::<Vec<_>>();
    let candidates =
        model_candidates(&json!({ "models": models }), Some("openrouter/vendor/model-399"))
            .unwrap();
    assert_eq!(candidates.len(), model_catalog::MAX_CATALOG_ENTRIES);
    assert_eq!(candidates[0].id, "openrouter/vendor/model-399");
}
#[test]
fn catalog_and_execution_policy_fail_closed() {
    for data in [
        json!({}),
        json!({"models":[{"provider":"bad/provider","id":"one"}]}),
        json!({"models":[{"provider":"p","id":"m"},{"provider":"p","id":"m"}]}),
    ] {
        assert!(model_candidates(&data, None).is_err());
    }
    let mut request = request("/tmp");
    for mode in [WriteMode::ReadOnly, WriteMode::Shared, WriteMode::Isolated] {
        request.write_mode = Some(mode);
        assert!(validate_scope(request).is_err());
    }
    request.write_mode = Some(WriteMode::Full);
    assert!(validate_scope(request).is_ok());
    assert_eq!(SANDBOXES, [SandboxMode::DangerFullAccess]);
    assert!(!CAPABILITIES.contains(&"briefings"));
    let args = launch_args(
        Path::new("/gate"),
        Path::new("/sessions"),
        request,
        None,
        true,
    );
    for flag in [
        "--no-approve",
        "--offline",
        "--no-extensions",
        "--no-mcp",
        "--no-context-files",
        "--no-skills",
        "--no-prompt-templates",
        "--no-themes",
        "--no-tools",
        "--no-session",
        "--session-dir",
    ] {
        assert!(args.contains(&flag.to_owned()));
    }
    assert!(!args.contains(&"--approve".into()));
}

#[test]
#[ignore = "requires BRIDGE_PI_LIVE_BINARY pointing to an installed Pi 1.x; no provider call is made"]
fn live_pi_rpc_proves_the_permission_gate_and_catalog_without_a_model_call() {
    let executable = std::env::var_os("BRIDGE_PI_LIVE_BINARY")
        .map(PathBuf::from)
        .expect("BRIDGE_PI_LIVE_BINARY");
    let profile = PiAdapter::discover(&executable).unwrap();
    assert!(profile.version.starts_with("1."));
}

#[cfg(unix)]
#[test]
fn stalled_stdin_has_a_deadline_and_shutdown_can_reap_the_group() {
    let (directory, path) = fake("normal");
    let cwd = directory.path().to_str().unwrap();
    let (mut runtime, _reader) =
        PiRuntime::open(&path, directory.path(), request(cwd), None, true).unwrap();
    runtime
        .process
        .client
        .command(json!({"type":"stop-reading"}), Duration::from_secs(1))
        .unwrap();
    let started = Instant::now();
    let result = runtime.process.client.command(
        json!({"type":"prompt","message":"x".repeat(1024*1024)}),
        Duration::from_millis(100),
    );
    assert!(result.is_err());
    assert!(started.elapsed() < Duration::from_secs(3));
    runtime.stop(ShutdownReason::AppShutdown);
    assert!(runtime.process.child.try_wait().unwrap().is_some());
}

#[test]
fn pi_usage_reaches_the_ledger_with_exclusive_cache_buckets_and_identity() {
    let mut state = normalize::StreamState::default();
    let events=state.normalize(&json!({"type":"message_end","message":{"role":"assistant","timestamp":42,"provider":"local","model":"one","content":[],"usage":{"input":12,"output":8,"cacheRead":3,"cacheWrite":4,"totalTokens":27,"cost":{"total":0.01}},"stopReason":"stop"}}));
    let data = &events
        .iter()
        .find(|event| event.kind == "usage.updated")
        .unwrap()
        .data;
    let records = crate::usage::normalize("provider.pi", data);
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.tokens.uncached_input_tokens, Some(12));
    assert_eq!(record.tokens.total_input_tokens, Some(19));
    assert_eq!(record.tokens.cache_read_tokens, Some(3));
    assert_eq!(record.tokens.cache_write_tokens, Some(4));
    assert_eq!(record.tokens.provider_reported_total_tokens, Some(27));
    assert_eq!(record.reported_cost_microusd, Some(10000));
    assert_eq!(record.model.as_deref(), Some("local/one"));
    assert_eq!(record.provider_record_id.as_deref(), Some("pi-message-42"));
}

#[test]
fn retry_errors_cannot_finish_a_run_until_settled() {
    let mut state = normalize::StreamState::default();
    state.normalize(&json!({"type":"agent_start"}));
    let events=state.normalize(&json!({"type":"message_end","message":{"role":"assistant","timestamp":42,"content":[],"stopReason":"error","errorMessage":"SECRET"}}));
    assert!(events.iter().all(|event| !(event.kind == "error"
        && event.status.as_deref() == Some("failed"))
        && event.kind != "turn.completed"));
    assert!(state
        .normalize(&json!({"type":"agent_end","willRetry":true}))
        .is_empty());
    assert!(state
        .normalize(&json!({"type":"auto_retry_end","success":false,"finalError":"SECRET"}))
        .iter()
        .all(|event| event.kind != "error" || event.status.as_deref() == Some("retrying")));
    let settled = state.normalize(&json!({"type":"agent_settled","aborted":false}));
    assert_eq!(settled[0].kind, "turn.completed");
    assert_eq!(settled[0].status.as_deref(), Some("failed"));
    assert_eq!(settled[1].kind, "error");
    assert!(!format!("{settled:?}").contains("SECRET"));
}

#[cfg(unix)]
#[test]
fn registry_and_config_use_the_proven_pi_adapter_and_exact_model_selection() {
    let (_directory, path) = fake("normal");
    let profile = PiAdapter::discover(&path).unwrap();
    let adapter = PiAdapter {
        profile: Arc::new(RwLock::new(Some(Ok(profile)))),
        discovering: Arc::new(AtomicBool::new(false)),
        rediscover: Arc::new(AtomicBool::new(false)),
        notify: None,
        streams: Mutex::new(HashMap::new()),
    };
    let descriptor = adapter.descriptor();
    assert!(descriptor.available);
    // Onboarding requires signed_in; a usable model proves Pi's provider auth.
    assert_eq!(descriptor.auth_state, AuthState::SignedIn);
    assert_eq!(descriptor.sandbox_modes, SANDBOXES);
    let mut registry = crate::adapters::AdapterRegistry::empty();
    registry.register(Box::new(adapter)).unwrap();
    assert!(registry.supports_native_resume("pi"));
    assert!(registry.supports_native_compaction("pi"));
    let db = crate::store::open(Path::new(":memory:")).unwrap();
    let mut harness = crate::agent_config::state(&db)
        .unwrap()
        .harnesses
        .into_iter()
        .find(|harness| harness.id == "pi")
        .unwrap();
    harness.default_model = Some("local/one".into());
    harness.effort = Some(crate::delegation::Effort::High);
    crate::agent_config::save_harness(&db, harness).unwrap();
    assert_eq!(
        crate::model::Harness::parse("pi").unwrap(),
        crate::model::Harness::Pi
    );
    assert!(!crate::adapters::descriptor_supports_agent_role(
        &descriptor,
        "research"
    ));
    assert!(!crate::adapters::descriptor_supports_agent_role(
        &descriptor,
        "implementation"
    ));
    let mut req = request("/tmp");
    req.model = Some("local/unavailable");
    assert!(registry.start("pi", req).is_err());
}

#[test]
fn delayed_duplicate_completion_does_not_double_count_between_runs() {
    let mut state = normalize::StreamState::default();
    let old = json!({"type":"message_end","message":{"role":"assistant","timestamp":1,"content":[{"type":"text","text":"Old"}],"usage":{"input":1,"output":1},"stopReason":"stop"}});
    state.normalize(&json!({"type":"agent_start"}));
    assert!(!state.normalize(&old).is_empty());
    state.normalize(&json!({"type":"agent_settled"}));
    state.normalize(&json!({"type":"agent_start"}));
    assert!(state.normalize(&old).is_empty());
    let recovering =
        state.normalize(&json!({"type":"compaction_end","aborted":true,"willRetry":true}));
    assert!(recovering.iter().all(|event| !(event.kind == "error"
        && event.status.as_deref() == Some("failed"))
        && event.kind != "turn.completed"));
}

#[cfg(unix)]
fn version_script(dir: &Path, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("pi-launcher");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(unix)]
#[test]
fn a_lazy_launcher_that_installs_on_first_run_is_still_found() {
    // Omarchy's ~/.local/bin/pi runs `mise use -g` (an install, the first
    // time) before exec'ing the tool, so the first version read is slow.
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("installed");
    let pi = version_script(dir.path(), &format!(
        "if [ ! -f '{m}' ]; then touch '{m}'; sleep 2; fi\necho 'pi 1.4.0'",
        m = marker.display()
    ));
    let cwd = dir.path().to_str().unwrap();
    let version = version_with_retry(&pi, cwd, Duration::from_millis(300), Duration::from_secs(20)).unwrap();
    assert_eq!(version, "1.4.0");
}

#[cfg(unix)]
#[test]
fn a_failing_version_probe_is_not_retried() {
    let dir = tempfile::tempdir().unwrap();
    let count = dir.path().join("count");
    let pi = version_script(dir.path(), &format!("echo x >> '{}'\nexit 3", count.display()));
    let cwd = dir.path().to_str().unwrap();
    assert!(version_with_retry(&pi, cwd, Duration::from_secs(5), Duration::from_secs(5)).is_err());
    assert_eq!(std::fs::read_to_string(&count).unwrap().lines().count(), 1);
}

#[cfg(unix)]
#[test]
fn a_launcher_that_never_answers_still_times_out() {
    let dir = tempfile::tempdir().unwrap();
    let pi = version_script(dir.path(), "sleep 30");
    let cwd = dir.path().to_str().unwrap();
    let started = Instant::now();
    assert!(version_with_retry(&pi, cwd, Duration::from_millis(200), Duration::from_millis(400)).is_err());
    assert!(started.elapsed() < Duration::from_secs(10));
}

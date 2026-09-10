//! Socket-level integration tests: a protocol client driving a real daemon —
//! ownership, handshake enforcement, typed dispatch, events, and recovery.

use bridged::{Daemon, DaemonConfig, StartupError, TOKEN_FILE_NAME};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// Point OpenCode at a nonexistent executable so background discovery fails
/// immediately instead of spawning a real OpenCode server.
fn seed_data_dir(data_dir: &Path) {
    let db = bridge_core::store::open(&data_dir.join("bridge.db")).unwrap();
    let mut opencode = bridge_core::agent_config::state(&db)
        .unwrap()
        .harnesses
        .into_iter()
        .find(|harness| harness.id == "opencode")
        .unwrap();
    opencode.advanced = serde_json::json!({
        "executablePath": data_dir.join("missing-opencode").to_string_lossy(),
    });
    bridge_core::agent_config::save_harness(&db, opencode).unwrap();
}

struct RunningDaemon {
    daemon: std::sync::Arc<Daemon>,
    accept_loop: Option<std::thread::JoinHandle<()>>,
    socket_path: PathBuf,
    token: String,
}

impl RunningDaemon {
    fn start(data_dir: &Path) -> RunningDaemon {
        seed_data_dir(data_dir);
        let (daemon, listener) = Daemon::start(DaemonConfig {
            data_dir: data_dir.to_path_buf(),
            socket_path: None,
            // No HTTP listener in tests: parallel tests must not fight over
            // a fixed port. The health surface has its own unit coverage.
            health_addr: None,
            browser_extension_path: data_dir.join("no-extension"),
            handshake_timeout: Duration::from_millis(400),
        })
        .expect("daemon starts");
        let daemon = std::sync::Arc::new(daemon);
        let socket_path = daemon.socket_path.clone();
        let token = daemon.state.auth_token.clone();
        let serve_daemon = daemon.clone();
        let accept_loop = std::thread::spawn(move || {
            bridged::serve(&serve_daemon, listener).expect("accept loop serves");
        });
        RunningDaemon { daemon, accept_loop: Some(accept_loop), socket_path, token }
    }

    fn stop(mut self) {
        self.daemon.state.shutting_down.store(true, Ordering::SeqCst);
        if let Some(handle) = self.accept_loop.take() {
            handle.join().unwrap();
        }
        self.daemon.shutdown(Duration::from_secs(5));
    }
}

struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Client {
    fn connect(socket_path: &Path) -> Client {
        let stream = UnixStream::connect(socket_path).expect("client connects");
        stream.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        Client { reader: BufReader::new(stream.try_clone().unwrap()), writer: stream }
    }

    fn send(&mut self, frame: Value) {
        let mut line = serde_json::to_vec(&frame).unwrap();
        line.push(b'\n');
        self.writer.write_all(&line).unwrap();
        self.writer.flush().unwrap();
    }

    fn recv(&mut self) -> Value {
        let mut line = String::new();
        self.reader.read_line(&mut line).expect("a frame arrives");
        assert!(!line.is_empty(), "the daemon closed the connection unexpectedly");
        serde_json::from_str(&line).expect("frames are JSON")
    }

    /// Receive frames until the response with `id` arrives, collecting any
    /// interleaved notifications.
    fn recv_response(&mut self, id: impl Into<Value>) -> (Value, Vec<Value>) {
        let id = id.into();
        let mut notifications = Vec::new();
        loop {
            let frame = self.recv();
            if frame.get("id").is_none() {
                assert_eq!(frame["jsonrpc"], json!("2.0"));
                assert!(
                    frame["method"].is_string()
                        && frame.get("result").is_none()
                        && frame.get("error").is_none(),
                    "expected a notification, got {frame:?}"
                );
                notifications.push(frame);
                continue;
            }
            assert_eq!(frame["id"], id, "responses arrive in request order");
            return (frame, notifications);
        }
    }

    fn handshake(&mut self, token: &str) -> Value {
        self.send(json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "protocol/handshake",
            "params": {
                "protocolVersion": {
                    "major": bridge_protocol::PROTOCOL_VERSION.major,
                    "minor": bridge_protocol::PROTOCOL_VERSION.minor,
                },
                "client": {"name": "daemon-test", "version": "0"},
                "authToken": token,
            },
        }));
        self.recv()
    }

    fn call(&mut self, id: i64, method: &str, params: Option<Value>) -> (Value, Vec<Value>) {
        let mut frame = json!({"jsonrpc": "2.0", "id": id, "method": method});
        if let Some(params) = params {
            frame["params"] = params;
        }
        self.send(frame);
        self.recv_response(id)
    }
}

#[test]
fn browser_frame_polling_uses_the_typed_daemon_contract() {
    let fixture = tempfile::tempdir().unwrap();
    let running = RunningDaemon::start(fixture.path());
    let mut client = Client::connect(&running.socket_path);
    assert!(client.handshake(&running.token).get("error").is_none());
    let (state, _) = client.call(1, "browser/browser_bridge_state", None);
    assert!(state.get("error").is_none());
    assert!(state["result"]["screenshot"].is_null());
    let (frame, _) = client.call(2, "browser/browser_frame", Some(json!({"afterRevision": 0})));
    assert!(frame.get("error").is_none());
    assert!(frame.get("result").is_some_and(Value::is_null));
    let (invalid, _) = client.call(3, "browser/browser_frame", Some(json!({"afterRevision": -1})));
    assert_eq!(invalid["error"]["code"], json!(-32602));
    drop(client);
    running.stop();
}

#[test]
fn menu_settings_persist_and_unknown_usage_stays_unknown_over_the_socket() {
    let fixture = tempfile::tempdir().unwrap();
    let running = RunningDaemon::start(fixture.path());
    let mut client = Client::connect(&running.socket_path);
    assert!(client.handshake(&running.token).get("error").is_none());
    let (initial, _) = client.call(1, "menu_bar/get_menu_bar_settings", None);
    let mut settings = initial["result"].clone();
    assert_eq!(settings["schemaVersion"], 1);
    settings["displayMode"] = json!("used");
    settings["enabled"] = json!(false);
    settings["claudeEnabled"] = json!(true);
    settings["cursorEnabled"] = json!(true);
    settings["opencodeEnabled"] = json!(true);
    settings["selectedProvider"] = json!("cursor");
    settings["opencodeWorkspace"] = json!("wrk_fixture");
    let (saved, _) = client.call(2, "menu_bar/save_menu_bar_settings", Some(json!({"settings": settings})));
    assert_eq!(saved["result"], settings);

    let mut invalid = settings.clone();
    invalid["refreshSeconds"] = json!(1);
    let (rejected, _) = client.call(3, "menu_bar/save_menu_bar_settings", Some(json!({"settings": invalid})));
    assert!(rejected.get("error").is_some());
    let (usage, _) = client.call(4, "usage/get_usage_overview", None);
    assert_eq!(usage["result"]["schemaVersion"], 1);
    assert_eq!(usage["result"]["windows"][0]["usedPercent"]["status"], "unavailable");
    assert!(usage["result"]["windows"][0]["usedPercent"]["value"].is_null());
    assert!(usage["result"]["today"]["costMicrousd"]["value"].is_null());
    let (group, _) = client.call(6, "usage/get_provider_usage_overviews", None);
    assert!(group.get("error").is_none());
    let rows = group["result"]["providers"].as_array().unwrap();
    assert_eq!(rows.iter().map(|r| r["provider"].as_str().unwrap()).collect::<Vec<_>>(), vec!["codex", "claude", "cursor", "opencode"]);
    for row in rows { assert!(row["today"]["tokens"]["value"].is_null()); }
    drop(client);
    running.stop();

    let restarted = RunningDaemon::start(fixture.path());
    let mut client = Client::connect(&restarted.socket_path);
    assert!(client.handshake(&restarted.token).get("error").is_none());
    let (loaded, _) = client.call(5, "menu_bar/get_menu_bar_settings", None);
    assert_eq!(loaded["result"], settings);
    drop(client);
    restarted.stop();
}

#[test]
fn a_session_is_created_driven_and_observed_end_to_end_over_the_socket() {
    let fixture = tempfile::tempdir().unwrap();
    let running = RunningDaemon::start(fixture.path());
    let mut client = Client::connect(&running.socket_path);

    let handshake = client.handshake(&running.token);
    assert_eq!(handshake["result"]["server"]["name"], json!("bridge"));
    assert!(handshake["result"]["capabilities"]
        .as_array()
        .unwrap()
        .contains(&json!("sessions")));
    // The daemon names its own binary, so a launcher holding a newer build
    // can tell this daemon is stale. The id is the test binary's here — what
    // matters on the wire is presence and shape.
    let build_id = handshake["result"]["buildId"]
        .as_str()
        .expect("the daemon reports the build identity of its own executable");
    assert_eq!(build_id.len(), 16, "a fixed-width hex identity: {build_id}");
    assert_eq!(
        build_id,
        bridge_core::binary::self_identity().unwrap(),
        "this in-process daemon and this test share an executable"
    );

    // Observe: the daemon read the token from the file it wrote.
    let on_disk =
        std::fs::read_to_string(fixture.path().join(TOKEN_FILE_NAME)).unwrap();
    assert_eq!(on_disk.trim(), running.token);

    let (health, _) = client.call(1, "health/health", None);
    assert_eq!(health["result"]["ok"], json!(true));

    // Create: a direct shell chat needs no provider binary.
    let (created, _) = client.call(2, "sessions/create_chat", Some(json!({"harness": "shell"})));
    let sessions = created["result"]["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 1);
    let session_id = sessions[0]["id"].as_str().unwrap().to_owned();

    // Observe: events published on the core bus reach this connection as
    // JSON-RPC notifications, refetch hints and durable agent events alike.
    // (That mutations publish after commit is bridge-core's own coverage.)
    running.daemon.core.events.publish(bridge_core::events::CoreEvent::StateChanged);
    running
        .daemon
        .core
        .events
        .publish(bridge_core::events::CoreEvent::Agent(bridge_core::model::AgentEvent {
            id: 1,
            session_id: session_id.clone(),
            sequence: 1,
            protocol_version: 1,
            kind: "message.completed".into(),
            item_id: None,
            role: Some("assistant".into()),
            status: Some("completed".into()),
            title: None,
            text: Some("hello from the daemon".into()),
            data: json!({}),
            provider_meta: json!({}),
            created_at: "now".into(),
        }));
    let hint = client.recv();
    assert_eq!(hint["method"], json!("state-changed"));
    assert!(hint.get("params").is_none(), "a null payload stays off the wire");
    let agent = client.recv();
    assert_eq!(agent["method"], json!("agent-event"));
    assert_eq!(agent["params"]["sessionId"], json!(session_id));
    assert_eq!(agent["params"]["sequence"], json!(1), "durable events carry their cursor");

    // Replay: the durable history for a fresh chat is empty, from cursor 0.
    let (replayed, _) = client.call(
        5,
        "sessions/replay_session_events",
        Some(json!({"sessionId": session_id, "afterSequence": 0})),
    );
    assert_eq!(replayed["result"], json!([]));

    // Drive: sending a turn to a chat that has not started fails inside the
    // runtime — proving dispatch reached the live-turn kernel, not a stub.
    let (turn, _) = client.call(
        6,
        "sessions/send_turn",
        Some(json!({"sessionId": session_id, "text": "hello"})),
    );
    let error = &turn["error"];
    assert!(error.is_object(), "send_turn without a started adapter errors: {turn}");
    assert!(error["code"].as_i64().unwrap() >= 1000, "a runtime code, not an envelope code");

    running.stop();
}

#[test]
fn the_handshake_gate_rejects_wrong_tokens_versions_and_orderings() {
    let fixture = tempfile::tempdir().unwrap();
    let running = RunningDaemon::start(fixture.path());

    // Wrong token → 2001 unauthorized.
    let mut client = Client::connect(&running.socket_path);
    let rejected = client.handshake("not-the-token");
    assert_eq!(rejected["error"]["code"], json!(2001));

    // Missing token → 2001 as well.
    let mut client = Client::connect(&running.socket_path);
    client.send(json!({
        "jsonrpc": "2.0", "id": 0, "method": "protocol/handshake",
        "params": {
            "protocolVersion": {"major": 0, "minor": 0},
            "client": {"name": "t", "version": "0"},
        },
    }));
    assert_eq!(client.recv()["error"]["code"], json!(2001));

    // Incompatible major → 2000, and the rejection names both versions.
    let mut client = Client::connect(&running.socket_path);
    client.send(json!({
        "jsonrpc": "2.0", "id": 0, "method": "protocol/handshake",
        "params": {
            "protocolVersion": {"major": bridge_protocol::PROTOCOL_VERSION.major + 1, "minor": 0},
            "client": {"name": "t", "version": "0"},
            "authToken": running.token,
        },
    }));
    let rejected = client.recv();
    assert_eq!(rejected["error"]["code"], json!(2000));
    assert!(rejected["error"]["data"]["serverProtocolVersion"].is_object());

    // Any other first request → invalid_request; the connection closes.
    let mut client = Client::connect(&running.socket_path);
    client.send(json!({"jsonrpc": "2.0", "id": 9, "method": "health/health"}));
    let rejected = client.recv();
    assert_eq!(rejected["error"]["code"], json!(-32600));

    // A second handshake on an authenticated connection is invalid too.
    let mut client = Client::connect(&running.socket_path);
    client.handshake(&running.token);
    let again = client.handshake(&running.token);
    assert_eq!(again["error"]["code"], json!(-32600));

    running.stop();
}

#[test]
fn dispatch_validates_params_against_the_contract() {
    let fixture = tempfile::tempdir().unwrap();
    let running = RunningDaemon::start(fixture.path());
    let mut client = Client::connect(&running.socket_path);
    client.handshake(&running.token);

    // Unknown method.
    let (response, _) = client.call(1, "sessions/no_such_method", None);
    assert_eq!(response["error"]["code"], json!(-32601));

    // Missing required field.
    let (response, _) = client.call(2, "workspaces/create_workspace", Some(json!({})));
    assert_eq!(response["error"]["code"], json!(-32602));

    // snake_case where the wire is camelCase.
    let (response, _) = client.call(
        3,
        "sessions/replay_session_events",
        Some(json!({"session_id": "s", "after_sequence": 0})),
    );
    assert_eq!(response["error"]["code"], json!(-32602));

    // Malformed harness id. `harness` stopped being a closed enum when the
    // marketplace opened it, so a *well-formed* id Bridge cannot run — like
    // `cursor`, a real registry agent — parses here and fails later with an
    // error naming the harness. Only a malformed one is `invalid_params`.
    let (response, _) = client.call(4, "sessions/create_chat", Some(json!({"harness": "Cursor"})));
    assert_eq!(response["error"]["code"], json!(-32602));

    // Unknown field on an otherwise valid payload — 0.5-era contracts reject
    // fields they do not name (pre-0.5 schemas stay open for compatibility).
    let (response, _) = client.call(
        5,
        "terminal/open_terminal",
        Some(json!({"workspaceId": "w", "profile": "zsh"})),
    );
    assert_eq!(response["error"]["code"], json!(-32602));

    // Params sent to a contracted-parameterless method.
    let (response, _) = client.call(6, "state/get_state", Some(json!({"verbose": true})));
    assert_eq!(response["error"]["code"], json!(-32602));

    // The connection survives every rejection above.
    let (health, _) = client.call(7, "health/health", None);
    assert_eq!(health["result"]["ok"], json!(true));

    running.stop();
}

#[test]
fn a_harness_unknown_at_compile_time_round_trips_entirely_over_rpc() {
    // The acceptance criterion for opening `HarnessId`: an id that no build of
    // Bridge has ever named must survive create -> persist -> replay ->
    // snapshot, driven over the socket with no desktop app running.
    let fixture = tempfile::tempdir().unwrap();
    let running = RunningDaemon::start(fixture.path());
    let mut client = Client::connect(&running.socket_path);
    client.handshake(&running.token);

    let (created, _) = client.call(
        1,
        "sessions/create_chat",
        Some(json!({"harness": "gemini", "title": "Gemini"})),
    );
    let sessions = created["result"]["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 1);
    let session_id = sessions[0]["id"].as_str().unwrap().to_owned();
    assert_eq!(
        sessions[0]["harness"],
        json!("gemini"),
        "the id comes back exactly as it was sent — the agent, not how it is run"
    );

    // Persisted, not merely echoed: a fresh read of the snapshot agrees.
    let (state, _) = client.call(2, "state/get_state", None);
    let persisted = state["result"]["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["id"] == json!(session_id))
        .expect("the session is in the snapshot");
    assert_eq!(persisted["harness"], json!("gemini"));

    // Replay is reachable for it like any other session.
    let (replayed, _) = client.call(
        3,
        "sessions/replay_session_events",
        Some(json!({"sessionId": session_id, "afterSequence": 0})),
    );
    assert!(replayed["result"].is_array(), "{replayed}");

    // Starting it fails by naming the harness, not by quietly running another
    // one. Nothing installs agents yet, so every non-built-in id is
    // uninstalled here — exactly the uninstalled-agent case.
    let (start, _) = client.call(4, "sessions/start_chat", Some(json!({"sessionId": session_id})));
    let message = start["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("gemini"),
        "starting an uninstalled harness must name it: {start}"
    );

    // A malformed id is refused; a well-formed one Bridge cannot run is not a
    // parse error. Naming and availability are different questions.
    let (rejected, _) = client.call(5, "sessions/create_chat", Some(json!({"harness": "Gemini"})));
    assert_eq!(rejected["error"]["code"], json!(-32602));
    assert!(rejected["error"]["message"].as_str().unwrap().contains("Gemini"));

    // A registry agent that shares a built-in's name is that built-in — one
    // agent, one id, one history, never two competing entries.
    let (builtin, _) = client.call(6, "sessions/create_chat", Some(json!({"harness": "opencode"})));
    let harnesses: Vec<&str> = builtin["result"]["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|session| session["harness"].as_str().unwrap())
        .collect();
    assert!(harnesses.contains(&"opencode"), "{harnesses:?}");
    assert!(harnesses.contains(&"gemini"), "{harnesses:?}");

    running.stop();
}

#[test]
fn a_second_owner_of_the_data_directory_is_refused_with_identity() {
    let fixture = tempfile::tempdir().unwrap();
    let running = RunningDaemon::start(fixture.path());

    let refused = Daemon::start(DaemonConfig {
        data_dir: fixture.path().to_path_buf(),
        socket_path: Some(fixture.path().join("second.sock")),
        health_addr: None,
        browser_extension_path: fixture.path().join("no-extension"),
        handshake_timeout: Duration::from_millis(400),
    });
    match refused {
        Err(StartupError::Ownership(error)) => {
            let message = error.to_string();
            assert!(message.contains("a bridged daemon"), "{message}");
            assert!(message.contains(&std::process::id().to_string()), "{message}");
        }
        Err(other) => panic!("a second daemon must fail on the lease, got {other}"),
        Ok(_) => panic!("a second daemon must fail on the lease, but it started"),
    }

    running.stop();
}

#[test]
fn a_killed_daemons_interrupted_turn_is_surfaced_after_restart() {
    let fixture = tempfile::tempdir().unwrap();
    let data_dir = fixture.path();

    // Simulate the aftermath of kill -9 mid-turn: a session mid-`working`
    // with a tracked adapter PID and an active turn, exactly what a dead
    // daemon leaves in SQLite.
    seed_data_dir(data_dir);
    let orphan = {
        let db = bridge_core::store::open(&data_dir.join("bridge.db")).unwrap();
        db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/bridged-test','now')", []).unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Task','bridge/task','/tmp/bridged-test-w','working','now')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,active_turn_id) VALUES('s','w','codex','Session','working','reported','turn')", []).unwrap();
        let mut command = std::process::Command::new("sleep");
        command.arg("30");
        bridge_core::adapters::configure_process_group(&mut command);
        let child = command.spawn().unwrap();
        bridge_core::session_supervisor::SessionSupervisor::track_adapter_process(
            &db,
            "s",
            child.id(),
        )
        .unwrap();
        child
    };

    // The "restarted" daemon boots on the same directory: the lease is free
    // (the old owner is dead), and boot-time recovery cleans up.
    let (daemon, listener) = Daemon::start(DaemonConfig {
        data_dir: data_dir.to_path_buf(),
        socket_path: None,
        health_addr: None,
        browser_extension_path: data_dir.join("no-extension"),
        handshake_timeout: Duration::from_millis(400),
    })
    .expect("restart acquires the dead owner's directory");
    let daemon = std::sync::Arc::new(daemon);
    let socket_path = daemon.socket_path.clone();
    let token = daemon.state.auth_token.clone();
    let serve_daemon = daemon.clone();
    let accept_loop = std::thread::spawn(move || {
        bridged::serve(&serve_daemon, listener).unwrap();
    });
    drop(orphan);

    let mut client = Client::connect(&socket_path);
    client.handshake(&token);
    let (state, _) = client.call(1, "state/get_state", None);
    let session = &state["result"]["sessions"][0];
    assert_eq!(session["id"], json!("s"));
    assert_eq!(
        session["status"],
        json!("stopped"),
        "the interrupted turn is surfaced as a stopped session, not left 'working'"
    );
    // Recovery also cleared the dead adapter's tracked PID and reconciled the
    // workspace off 'working' — the session is restartable, not wedged.
    assert_eq!(state["result"]["workspaces"][0]["status"], json!("ready"));

    daemon.state.shutting_down.store(true, Ordering::SeqCst);
    accept_loop.join().unwrap();
    daemon.shutdown(Duration::from_secs(5));
}

/// A real, tracked OS process without calling a live model provider.
struct ShutdownRuntime {
    child: std::process::Child,
    provider_id: String,
    reasons: std::sync::Arc<std::sync::Mutex<Vec<bridge_core::adapters::ShutdownReason>>>,
}

impl bridge_core::adapters::AdapterRuntime for ShutdownRuntime {
    fn process_id(&self) -> u32 { self.child.id() }
    fn provider_session_id(&self) -> &str { &self.provider_id }
    fn current_turn(&self) -> std::sync::Arc<std::sync::Mutex<Option<String>>> {
        Default::default()
    }
    fn send_turn(&self, _: &str) -> Result<(), bridge_core::BridgeError> { Ok(()) }
    fn interrupt(&self) -> Result<(), bridge_core::BridgeError> { Ok(()) }
    fn respond(&self, _: Value, _: &str) -> Result<(), bridge_core::BridgeError> { Ok(()) }
    fn stop(&mut self, reason: bridge_core::adapters::ShutdownReason) {
        self.reasons.lock().unwrap().push(reason);
        let _ = self.child.kill();
        self.child.wait().unwrap();
    }
}

impl Drop for ShutdownRuntime {
    fn drop(&mut self) {
        // Also reap the fixture when an assertion fails before daemon shutdown.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn clean_shutdown_retires_provider_claims_without_failing_finished_chats_on_restart() {
    let fixture = tempfile::tempdir().unwrap();
    let running = RunningDaemon::start(fixture.path());
    let core = running.daemon.core.clone();
    let reasons = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let cases = [
        ("ready", "ready", "stopped"),
        ("working", "working", "stopped"),
        ("failed", "failed", "failed"),
        ("completed", "completed", "completed"),
        ("cancelled", "cancelled", "cancelled"),
        ("reader-exit", "ready", "stopped"),
    ];
    for (id, status, _) in cases {
        let provider_id = format!("native-{id}");
        let mut command = std::process::Command::new("sleep");
        command.arg("30");
        bridge_core::adapters::configure_process_group(&mut command);
        let mut runtime = ShutdownRuntime {
            child: command.spawn().unwrap(),
            provider_id: provider_id.clone(),
            reasons: reasons.clone(),
        };
        {
            let db = core.db.lock().unwrap();
            db.execute(
                "INSERT INTO sessions(id,workspace_id,harness,label,status,kind,metric_source,provider_session_id,active_turn_id)
                 VALUES(?1,NULL,'codex',?1,?4,'direct','reported',?2,?3)",
                rusqlite::params![id, provider_id, (status == "working").then_some("active-turn"), status],
            ).unwrap();
            bridge_core::session_forest::SessionForest::new(&db).append(
                id,
                bridge_core::session_forest::EntryKind::AssistantMessage,
                json!({"text": "The previous answer survives shutdown."}),
            ).unwrap();
            bridge_core::session_supervisor::SessionSupervisor::track_adapter_process(
                &db, id, runtime.child.id(),
            ).unwrap();
        }
        if id == "reader-exit" {
            // The reader already removed the runtime, but its durable cleanup
            // has not run yet when shutdown takes its snapshot.
            runtime.child.kill().unwrap();
            runtime.child.wait().unwrap();
        } else {
            core.adapters.lock().unwrap().insert(id.into(), Box::new(runtime));
        }
    }

    running.stop();
    assert_eq!(reasons.lock().unwrap().as_slice(), &[bridge_core::adapters::ShutdownReason::AppShutdown; 5]);
    assert!(core.adapters.lock().unwrap().is_empty());
    {
        // Check before store::open can repair anything: shutdown itself must
        // commit the stopped state and remove both durable process claims.
        let db = core.db.lock().unwrap();
        for (id, previous, expected) in cases {
            let saved: (String, Option<String>, Option<i64>, Option<String>, String) = db.query_row(
                "SELECT status,active_turn_id,adapter_pid,adapter_process_identity,provider_session_id FROM sessions WHERE id=?1",
                [id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
            ).unwrap();
            assert_eq!(saved, (expected.into(), None, None, None, format!("native-{id}")));
            let entry = bridge_core::store::session_entries(&db, id).unwrap().pop().unwrap();
            assert_eq!(entry.payload["reason"], "app_shutdown");
            assert_eq!(entry.payload["interrupted"], previous == "working");
        }
    }
    drop(core);

    let restarted = RunningDaemon::start(fixture.path());
    {
        let db = restarted.daemon.core.db.lock().unwrap();
        for (id, _, expected) in cases {
            let status: String = db.query_row("SELECT status FROM sessions WHERE id=?1", [id], |row| row.get(0)).unwrap();
            assert_eq!(status, expected);
            let entries = bridge_core::store::session_entries(&db, id).unwrap();
            assert!(entries.iter().any(|entry| entry.kind == "assistant.message"
                && entry.payload["text"] == "The previous answer survives shutdown."));
        }
        let failures: i64 = db.query_row(
            "SELECT COUNT(*) FROM events WHERE kind='adapter.request_failed' OR kind LIKE 'adapter.orphan_%'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(failures, 0, "clean shutdown must not be diagnosed as orphan recovery");
    }
    restarted.stop();
}

#[test]
fn graceful_shutdown_does_not_leave_chats_as_failed_orphans_after_restart() {
    let fixture = tempfile::tempdir().unwrap();
    let running = RunningDaemon::start(fixture.path());
    let mut client = Client::connect(&running.socket_path);
    client.handshake(&running.token);
    let mut cases: Vec<(String, &str)> = Vec::new();
    for (index, status) in ["ready", "working", "waiting", "completed", "failed"].iter().enumerate() {
        let (created, _) = client.call(index as i64 + 1, "sessions/create_chat", Some(json!({"harness": "shell"})));
        let session_id = created["result"]["sessions"].as_array().unwrap().iter()
            .find(|session| !cases.iter().any(|(id, _)| session["id"].as_str() == Some(id.as_str())))
            .unwrap()["id"].as_str().unwrap().to_owned();
        {
            let db = running.daemon.core.db.lock().unwrap();
            db.execute(
                "UPDATE sessions SET status=?2,adapter_pid=2147483647,adapter_process_identity='test-provider',active_turn_id=?3 WHERE id=?1",
                rusqlite::params![session_id, status, if matches!(*status, "working" | "waiting") { Some("turn") } else { None }],
            ).unwrap();
        }
        running.daemon.core.adapters.lock().unwrap().insert(
            session_id.clone(),
            Box::new(RecordingRuntime { responded: Default::default() }),
        );
        cases.push((session_id, *status));
    }
    drop(client);
    running.stop();

    let restarted = RunningDaemon::start(fixture.path());
    {
        let db = restarted.daemon.core.db.lock().unwrap();
        for (session_id, previous_status) in cases {
            let (status, pid, identity, active_turn): (String, Option<i64>, Option<String>, Option<String>) = db.query_row(
                "SELECT status,adapter_pid,adapter_process_identity,active_turn_id FROM sessions WHERE id=?1",
                [&session_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            ).unwrap();
            let expected = if matches!(previous_status, "completed" | "failed") { previous_status } else { "stopped" };
            assert_eq!(status, expected, "status before shutdown: {previous_status}");
            assert_eq!((pid, identity, active_turn), (None, None, None));
            let entries = bridge_core::store::session_entries(&db, &session_id).unwrap();
            assert!(!entries.iter().any(|entry| entry.payload["reason"] == "supervisor_restart_orphan"),
                "an orderly stop must not be reported as an orphaned provider");
            if expected == "stopped" {
                assert!(entries.iter().any(|entry| entry.payload["reason"] == "app_shutdown"));
            }
        }
    }
    restarted.stop();
}

#[test]
fn oversized_frames_are_refused_with_a_bounded_error() {
    let fixture = tempfile::tempdir().unwrap();
    let running = RunningDaemon::start(fixture.path());
    let mut client = Client::connect(&running.socket_path);
    client.handshake(&running.token);

    // A background notification may precede the framing error. Leave one
    // buffered deliberately so this verifies response matching independently
    // of discovery/pricing startup timing.
    running.daemon.core.events.publish(bridge_core::events::CoreEvent::AdaptersChanged);
    assert!(!client.reader.fill_buf().expect("a notification arrives").is_empty());

    let huge = "x".repeat(bridged::MAX_FRAME_BYTES + 16);
    client.send(json!({
        "jsonrpc": "2.0", "id": 1, "method": "sessions/send_turn",
        "params": {"sessionId": "s", "text": huge},
    }));
    // The oversized body is never parsed, so its request id cannot be echoed.
    let (response, notifications) = client.recv_response(Value::Null);
    assert!(!notifications.is_empty(), "the error follows a valid notification");
    assert_eq!(response["error"]["code"], json!(-32600));
    assert!(response["error"]["message"].as_str().unwrap().contains("exceeds"));
    assert!(serde_json::to_vec(&response).unwrap().len() < 1024, "the error must not echo the oversized body");

    running.stop();
}

#[test]
fn shutdown_stops_accepting_and_removes_the_socket() {
    let fixture = tempfile::tempdir().unwrap();
    let running = RunningDaemon::start(fixture.path());
    let socket_path = running.socket_path.clone();
    assert!(socket_path.exists());

    running.stop();

    assert!(!socket_path.exists(), "shutdown removes the socket file");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match UnixStream::connect(&socket_path) {
            Err(_) => break,
            Ok(_) if Instant::now() > deadline => {
                panic!("connections must be refused after shutdown")
            }
            Ok(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

#[test]
fn connection_slots_are_released_after_clean_disconnects() {
    let fixture = tempfile::tempdir().unwrap();
    let running = RunningDaemon::start(fixture.path());

    // Churn more clean connections than the daemon has slots; each must give
    // its slot back promptly even though the event bus stays quiet.
    for round in 0..(bridged::MAX_CONNECTIONS + 4) {
        let mut client = Client::connect(&running.socket_path);
        let handshake = client.handshake(&running.token);
        assert!(handshake["result"].is_object(), "round {round}: {handshake}");
        drop(client);
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while running.daemon.state.connections.load(Ordering::SeqCst) > 0 {
        assert!(
            Instant::now() < deadline,
            "{} slot(s) still occupied after every client disconnected",
            running.daemon.state.connections.load(Ordering::SeqCst)
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    // And the daemon still serves.
    let mut client = Client::connect(&running.socket_path);
    client.handshake(&running.token);
    let (health, _) = client.call(1, "health/health", None);
    assert_eq!(health["result"]["ok"], json!(true));

    running.stop();
}

#[test]
fn a_silent_connection_is_reclaimed_at_the_handshake_deadline() {
    let fixture = tempfile::tempdir().unwrap();
    let running = RunningDaemon::start(fixture.path());

    // Connect and say nothing. The daemon (400ms handshake deadline in
    // tests) must reject and close rather than let the slot be parked on.
    let mut client = Client::connect(&running.socket_path);
    let started = Instant::now();
    let rejection = client.recv();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(rejection["error"]["code"], json!(-32600));
    assert!(rejection["error"]["message"].as_str().unwrap().contains("deadline"));
    let mut end = String::new();
    client.reader.read_line(&mut end).unwrap();
    assert!(end.is_empty(), "the connection closes after the deadline rejection");
    let deadline = Instant::now() + Duration::from_secs(5);
    while running.daemon.state.connections.load(Ordering::SeqCst) > 0 {
        assert!(Instant::now() < deadline, "the silent connection's slot never freed");
        std::thread::sleep(Duration::from_millis(20));
    }

    running.stop();
}

#[test]
fn cancellation_and_envelope_semantics_follow_json_rpc() {
    let fixture = tempfile::tempdir().unwrap();
    let running = RunningDaemon::start(fixture.path());
    let mut client = Client::connect(&running.socket_path);
    client.handshake(&running.token);

    // Request-form $/cancel is a malformed use of a notification: the daemon
    // must not claim success for a cancellation that cannot happen.
    let (response, _) = client.call(1, "$/cancel", Some(json!({"id": 99})));
    assert_eq!(response["error"]["code"], json!(-32600));
    assert!(response["error"]["message"].as_str().unwrap().contains("interrupt_turn"));

    // Notification-form $/cancel gets no response; the connection moves on.
    // Background discovery may publish a valid notification between any of
    // these replies. The response helper must still reject an extra response
    // (including one with a null id) without mistaking that event for a reply.
    running.daemon.core.events.publish(bridge_core::events::CoreEvent::AdaptersChanged);
    client.send(json!({"jsonrpc": "2.0", "method": "$/cancel", "params": {"id": 99}}));
    let (health, _) = client.call(2, "health/health", None);
    assert_eq!(health["result"]["ok"], json!(true));

    // A structurally invalid request echoes its id with invalid_request —
    // not a parse error with a null id.
    client.send(json!({"jsonrpc": "2.0", "id": 7, "params": {}}));
    let (response, _) = client.recv_response(7);
    assert_eq!(response["id"], json!(7));
    assert_eq!(response["error"]["code"], json!(-32600));

    // Non-JSON is the parse-error case, with the null id the spec requires.
    client.writer.write_all(b"not json at all\n").unwrap();
    client.writer.flush().unwrap();
    let (response, _) = client.recv_response(Value::Null);
    assert_eq!(response["id"], Value::Null);
    assert_eq!(response["error"]["code"], json!(-32700));

    running.stop();
}

#[test]
fn requests_after_shutdown_are_refused_with_their_own_id() {
    let fixture = tempfile::tempdir().unwrap();
    let running = RunningDaemon::start(fixture.path());
    let mut client = Client::connect(&running.socket_path);
    client.handshake(&running.token);

    running.daemon.state.shutting_down.store(true, Ordering::SeqCst);
    client.send(json!({"jsonrpc": "2.0", "id": 41, "method": "health/health"}));
    // The read loop may close the connection on an idle poll before reading
    // the frame; a refusal, when one arrives, must carry the request's id.
    let mut line = String::new();
    if client.reader.read_line(&mut line).is_ok() && !line.is_empty() {
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], json!(41));
        assert_eq!(response["error"]["code"], json!(2003));
    }

    running.stop();
}

/// An adapter runtime that records approval responses, standing in for a live
/// provider process.
struct RecordingRuntime {
    responded: std::sync::Arc<std::sync::Mutex<Vec<(Value, String)>>>,
}

impl bridge_core::adapters::AdapterRuntime for RecordingRuntime {
    fn process_id(&self) -> u32 {
        0
    }
    fn provider_session_id(&self) -> &str {
        "recording"
    }
    fn current_turn(&self) -> std::sync::Arc<std::sync::Mutex<Option<String>>> {
        std::sync::Arc::new(std::sync::Mutex::new(None))
    }
    fn send_turn(&self, _text: &str) -> Result<(), bridge_core::BridgeError> {
        Ok(())
    }
    fn interrupt(&self) -> Result<(), bridge_core::BridgeError> {
        Ok(())
    }
    fn respond(&self, request_id: Value, decision: &str) -> Result<(), bridge_core::BridgeError> {
        self.responded.lock().unwrap().push((request_id, decision.to_owned()));
        Ok(())
    }
    fn stop(&mut self, _reason: bridge_core::adapters::ShutdownReason) {}
}

#[test]
fn an_approval_is_resolved_end_to_end_over_the_socket() {
    let fixture = tempfile::tempdir().unwrap();
    let running = RunningDaemon::start(fixture.path());
    let mut client = Client::connect(&running.socket_path);
    client.handshake(&running.token);

    // Create the chat over the socket, then wire a recording adapter and a
    // pending approval into the runtime — the shape a live provider leaves
    // when it asks for permission mid-turn.
    let (created, _) = client.call(1, "sessions/create_chat", Some(json!({"harness": "shell"})));
    let session_id = created["result"]["sessions"][0]["id"].as_str().unwrap().to_owned();
    let responded = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    running.daemon.core.adapters.lock().unwrap().insert(
        session_id.clone(),
        Box::new(RecordingRuntime { responded: responded.clone() }),
    );
    let approval_sequence = {
        let db = running.daemon.core.db.lock().unwrap();
        let event = bridge_core::store::session_event(
            &db,
            &session_id,
            &bridge_core::agent::NormalizedEvent {
                kind: "approval.requested".into(),
                item_id: Some("tool-1".into()),
                role: Some("tool".into()),
                status: Some("pending".into()),
                title: Some("Run cargo test".into()),
                text: None,
                data: json!({"requestId": "approval-1"}),
            },
            &json!({"adapter": "shell"}),
        )
        .unwrap();
        event.sequence
    };

    // Approve over the socket.
    let (resolved, mut notifications) = client.call(
        2,
        "approvals/resolve_approval",
        Some(json!({
            "sessionId": session_id,
            "eventId": approval_sequence,
            "decision": "accept",
        })),
    );
    assert!(
        resolved.get("error").is_none() && resolved.get("result").is_some(),
        "resolve_approval must succeed: {resolved}"
    );

    // The decision reached the adapter…
    let responses = responded.lock().unwrap().clone();
    assert_eq!(responses, vec![(json!("approval-1"), "accept".to_owned())]);

    // …the resolution is durable and replayable from the cursor…
    let (replayed, more) = client.call(
        3,
        "sessions/replay_session_events",
        Some(json!({"sessionId": session_id, "afterSequence": approval_sequence})),
    );
    notifications.extend(more);
    let events = replayed["result"].as_array().unwrap();
    assert!(
        events.iter().any(|event| event["kind"] == json!("approval.resolved")
            && event["status"] == json!("accept")),
        "replay must include the resolution: {events:?}"
    );

    // …and the durable agent event was pushed to this connection live (it
    // may already have interleaved with the responses above).
    let resolution_pushed = |frame: &Value| {
        frame["method"] == json!("agent-event")
            && frame["params"]["kind"] == json!("approval.resolved")
            && frame["params"]["sessionId"] == json!(session_id)
    };
    while !notifications.iter().any(resolution_pushed) {
        // recv() panics via the read timeout if the notification never comes.
        notifications.push(client.recv());
    }

    running.stop();
}

#[test]
fn sigkill_of_a_live_daemon_frees_the_directory_and_preserves_state() {
    let fixture = tempfile::tempdir().unwrap();
    let data_dir = fixture.path();
    seed_data_dir(data_dir);

    // A real daemon process, killed for real: no in-process shortcuts.
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_bridged"))
        .args(["--data-dir", data_dir.to_str().unwrap(), "--health-addr", "none"])
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let socket_path = data_dir.join(bridged::SOCKET_FILE_NAME);
    let token_path = data_dir.join(TOKEN_FILE_NAME);
    let deadline = Instant::now() + Duration::from_secs(60);
    while !socket_path.exists() || !token_path.exists() {
        assert!(Instant::now() < deadline, "the daemon process never came up");
        std::thread::sleep(Duration::from_millis(50));
    }
    let token = std::fs::read_to_string(&token_path).unwrap().trim().to_owned();

    let mut client = Client::connect(&socket_path);
    let handshake = client.handshake(&token);
    assert!(handshake["result"].is_object(), "{handshake}");
    let (created, _) = client.call(1, "sessions/create_chat", Some(json!({"harness": "shell"})));
    let session_id = created["result"]["sessions"][0]["id"].as_str().unwrap().to_owned();

    // SIGKILL: no graceful path runs — no drain, no adapter teardown, no
    // socket cleanup, no lease release code.
    child.kill().unwrap();
    child.wait().unwrap();

    // The lease died with the process: a successor starts immediately, and
    // the killed daemon's committed state is intact.
    let running = RunningDaemon::start(data_dir);
    let mut client = Client::connect(&running.socket_path);
    client.handshake(&running.token);
    let (state, _) = client.call(1, "state/get_state", None);
    let sessions = state["result"]["sessions"].as_array().unwrap();
    assert!(
        sessions.iter().any(|session| session["id"] == json!(session_id)),
        "the chat created through the killed daemon survives: {sessions:?}"
    );

    running.stop();
}

#[test]
fn shutdown_drains_open_connections_before_completing() {
    let fixture = tempfile::tempdir().unwrap();
    let running = RunningDaemon::start(fixture.path());
    let mut client = Client::connect(&running.socket_path);
    client.handshake(&running.token);

    // An idle-but-open connection: the drain must not need the client's
    // cooperation — the read loop notices the flag on its next poll.
    let daemon = running.daemon.clone();
    daemon.state.shutting_down.store(true, Ordering::SeqCst);
    let drained_by = Instant::now() + Duration::from_secs(5);
    while daemon.state.connections.load(Ordering::SeqCst) > 0 {
        assert!(Instant::now() < drained_by, "the idle connection never drained");
        std::thread::sleep(Duration::from_millis(20));
    }

    running.stop();
}

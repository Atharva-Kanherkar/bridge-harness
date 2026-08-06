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
        self.daemon.shutdown();
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
    fn recv_response(&mut self, id: i64) -> (Value, Vec<Value>) {
        let mut notifications = Vec::new();
        loop {
            let frame = self.recv();
            if frame.get("id").is_none() {
                notifications.push(frame);
                continue;
            }
            assert_eq!(frame["id"], json!(id), "responses arrive in request order");
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

    // Unknown enum value.
    let (response, _) = client.call(4, "sessions/create_chat", Some(json!({"harness": "cursor"})));
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
fn a_second_owner_of_the_data_directory_is_refused_with_identity() {
    let fixture = tempfile::tempdir().unwrap();
    let running = RunningDaemon::start(fixture.path());

    let refused = Daemon::start(DaemonConfig {
        data_dir: fixture.path().to_path_buf(),
        socket_path: Some(fixture.path().join("second.sock")),
        health_addr: None,
        browser_extension_path: fixture.path().join("no-extension"),
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
    daemon.shutdown();
}

#[test]
fn oversized_frames_are_refused_with_a_bounded_error() {
    let fixture = tempfile::tempdir().unwrap();
    let running = RunningDaemon::start(fixture.path());
    let mut client = Client::connect(&running.socket_path);
    client.handshake(&running.token);

    let huge = "x".repeat(bridged::MAX_FRAME_BYTES + 16);
    client.send(json!({
        "jsonrpc": "2.0", "id": 1, "method": "sessions/send_turn",
        "params": {"sessionId": "s", "text": huge},
    }));
    let response = client.recv();
    assert_eq!(response["error"]["code"], json!(-32600));
    assert!(response["error"]["message"].as_str().unwrap().contains("exceeds"));

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

//! The #107 verification checklist, client-side: reconnect-and-replay after a
//! daemon restart, deterministic recovery from live-channel lag, a pending
//! approval surviving a reconnect, visible second-owner errors, and the exec
//! one-shot driving a real daemon binary-style flow end to end.

use bridge_client::{ClientError, DaemonClient, Endpoint, SessionEventStream};
use bridge_protocol::MethodName;
use serde_json::json;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

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
    daemon: std::sync::Arc<bridged::Daemon>,
    accept_loop: Option<std::thread::JoinHandle<()>>,
}

impl RunningDaemon {
    fn start(data_dir: &Path) -> RunningDaemon {
        let (daemon, listener) = bridged::Daemon::start(bridged::DaemonConfig {
            data_dir: data_dir.to_path_buf(),
            socket_path: None,
            health_addr: None,
            browser_extension_path: data_dir.join("no-extension"),
            handshake_timeout: Duration::from_secs(5),
        })
        .expect("daemon starts");
        let daemon = std::sync::Arc::new(daemon);
        let serve_daemon = daemon.clone();
        let accept_loop = std::thread::spawn(move || {
            bridged::serve(&serve_daemon, listener).expect("accept loop serves");
        });
        RunningDaemon { daemon, accept_loop: Some(accept_loop) }
    }

    fn stop(mut self) {
        self.daemon.state.shutting_down.store(true, Ordering::SeqCst);
        if let Some(handle) = self.accept_loop.take() {
            handle.join().unwrap();
        }
        self.daemon.shutdown(Duration::from_secs(5));
    }
}

fn connect(data_dir: &Path) -> DaemonClient {
    let mut client =
        DaemonClient::connect(&Endpoint::for_data_dir(data_dir).unwrap()).expect("client connects");
    client.set_call_timeout(Duration::from_secs(30));
    client
}

fn create_chat(client: &DaemonClient) -> String {
    let created = client
        .call(MethodName::CreateChatId, Some(json!({"harness": "shell"})))
        .unwrap();
    created["sessionId"].as_str().unwrap().to_owned()
}

/// Persist a durable event and publish it on the bus, as a live mutation does.
fn publish_durable(daemon: &bridged::Daemon, session_id: &str, text: &str) -> i64 {
    let event = {
        let db = daemon.core.db.lock().unwrap();
        bridge_core::store::session_event(
            &db,
            session_id,
            &bridge_core::agent::NormalizedEvent {
                kind: "message.completed".into(),
                item_id: None,
                role: Some("assistant".into()),
                status: Some("completed".into()),
                title: None,
                text: Some(text.into()),
                data: json!({}),
            },
            &json!({"adapter": "test"}),
        )
        .unwrap()
    };
    daemon
        .core
        .events
        .publish(bridge_core::events::CoreEvent::Agent(event.clone()));
    event.sequence
}

#[test]
fn a_daemon_restart_is_survived_by_reconnect_and_cursor_replay() {
    let fixture = tempfile::tempdir().unwrap();
    let data_dir = fixture.path();
    seed_data_dir(data_dir);

    // First daemon: stream the first two events live.
    let first = RunningDaemon::start(data_dir);
    let client = connect(data_dir);
    let session_id = create_chat(&client);
    let mut stream = SessionEventStream::new(&client, session_id.clone(), 0).unwrap();
    publish_durable(&first.daemon, &session_id, "one");
    publish_durable(&first.daemon, &session_id, "two");
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut seen = Vec::new();
    while seen.len() < 2 {
        let event = stream.next(deadline).unwrap().expect("live events arrive");
        if event.sequence > 0 {
            seen.push(event.sequence);
        }
    }
    let cursor = stream.cursor();

    // The daemon dies and is replaced; the old connection reports itself dead.
    first.stop();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match stream.next(deadline) {
            Err(ClientError::Disconnected) => break,
            Ok(_) => continue,
            Err(other) => panic!("expected a disconnect, got {other}"),
        }
    }
    let second = RunningDaemon::start(data_dir);
    // Events landed while this client was disconnected.
    publish_durable(&second.daemon, &session_id, "three");
    publish_durable(&second.daemon, &session_id, "four");

    // Reconnect and resume from the cursor: replay hands over exactly the
    // missed durable events, in order, no duplicates.
    let client = connect(data_dir);
    let mut resumed = SessionEventStream::new(&client, session_id.clone(), cursor).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut recovered = Vec::new();
    while recovered.len() < 2 {
        let event = resumed.next(deadline).unwrap().expect("replay recovers missed events");
        recovered.push((event.sequence, event.payload["text"].as_str().unwrap().to_owned()));
    }
    let sequences: Vec<i64> = recovered.iter().map(|(sequence, _)| *sequence).collect();
    assert_eq!(sequences, {
        let mut expected = seen.clone();
        expected.iter_mut().for_each(|sequence| *sequence += 2);
        expected
    }, "contiguous continuation of the pre-restart cursor");
    assert_eq!(
        recovered.iter().map(|(_, text)| text.as_str()).collect::<Vec<_>>(),
        ["three", "four"]
    );

    second.stop();
}

#[test]
fn live_channel_lag_is_recovered_deterministically_with_no_gaps() {
    let fixture = tempfile::tempdir().unwrap();
    let data_dir = fixture.path();
    seed_data_dir(data_dir);
    let running = RunningDaemon::start(data_dir);
    let client = connect(data_dir);
    let session_id = create_chat(&client);
    let mut stream = SessionEventStream::new(&client, session_id.clone(), 0).unwrap();

    // Far more durable events than the per-connection queue holds, published
    // while this client reads nothing: the daemon must drop live frames and
    // owe a stream-lagged marker.
    const TOTAL: i64 = 3000;
    for index in 0..TOTAL {
        publish_durable(&running.daemon, &session_id, &format!("event {index}"));
    }

    // Now drain: replayed and live events must interleave into exactly
    // 1..=TOTAL with no gaps and no duplicates.
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut next_expected = 1i64;
    while next_expected <= TOTAL {
        let event = stream
            .next(deadline)
            .unwrap()
            .unwrap_or_else(|| panic!("stalled waiting for sequence {next_expected}"));
        if event.sequence == 0 {
            continue;
        }
        assert_eq!(
            event.sequence, next_expected,
            "durable events arrive exactly once, in order"
        );
        next_expected += 1;
    }

    running.stop();
}

#[test]
fn a_pending_approval_survives_a_reconnect() {
    let fixture = tempfile::tempdir().unwrap();
    let data_dir = fixture.path();
    seed_data_dir(data_dir);

    let first = RunningDaemon::start(data_dir);
    let client = connect(data_dir);
    let session_id = create_chat(&client);
    let approval_sequence = {
        let db = first.daemon.core.db.lock().unwrap();
        bridge_core::store::session_event(
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
        .unwrap()
        .sequence
    };
    drop(client);
    first.stop();

    // After the restart, the approval is still pending on durable history —
    // recovered, not dropped.
    let second = RunningDaemon::start(data_dir);
    let client = connect(data_dir);
    let replayed = client.replay_session_events(&session_id, 0).unwrap();
    let pending: Vec<&serde_json::Value> = replayed
        .iter()
        .filter(|event| event["kind"] == json!("approval.requested"))
        .collect();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0]["sequence"].as_i64().unwrap(), approval_sequence);
    assert!(
        !replayed.iter().any(|event| event["kind"] == json!("approval.resolved")),
        "nothing resolved it behind the operator's back"
    );

    // And it is still actionable: resolving against the restarted daemon
    // reaches the adapter layer (which reports the process is gone — the
    // truthful state after a restart, not a dropped approval).
    let outcome = client.call(
        MethodName::ResolveApproval,
        Some(json!({
            "sessionId": session_id,
            "eventId": approval_sequence,
            "decision": "accept",
        })),
    );
    match outcome {
        Err(ClientError::Rpc(error)) => {
            assert!(
                error.message.contains("not running"),
                "the rejection names the dead adapter, got: {}",
                error.message
            );
        }
        other => panic!("expected the adapter-gone error, got {other:?}"),
    }

    second.stop();
}

#[test]
fn second_owners_are_rejected_with_visible_identity() {
    let fixture = tempfile::tempdir().unwrap();
    let data_dir = fixture.path();
    seed_data_dir(data_dir);
    let running = RunningDaemon::start(data_dir);

    let refused = bridged::Daemon::start(bridged::DaemonConfig {
        data_dir: data_dir.to_path_buf(),
        socket_path: Some(data_dir.join("second.sock")),
        health_addr: None,
        browser_extension_path: data_dir.join("no-extension"),
        handshake_timeout: Duration::from_secs(5),
    });
    let message = match refused {
        Err(error) => error.to_string(),
        Ok(_) => panic!("the lease must refuse a second owner"),
    };
    assert!(message.contains("a bridged daemon"), "{message}");
    assert!(message.contains(&std::process::id().to_string()), "{message}");

    running.stop();
}

#[test]
fn an_in_flight_turn_is_surfaced_stopped_after_graceful_shutdown() {
    let fixture = tempfile::tempdir().unwrap();
    let data_dir = fixture.path();
    seed_data_dir(data_dir);

    let first = RunningDaemon::start(data_dir);
    let client = connect(data_dir);
    let session_id = create_chat(&client);
    {
        // A turn mid-flight: working status, active turn id.
        let db = first.daemon.core.db.lock().unwrap();
        db.execute(
            "UPDATE sessions SET status='working', active_turn_id='turn-1' WHERE id=?1",
            rusqlite::params![session_id],
        )
        .unwrap();
    }
    drop(client);
    first.stop();

    // The successor surfaces the interrupted turn as stopped — cleanly
    // recoverable, not stuck 'working' with a dead owner.
    let second = RunningDaemon::start(data_dir);
    let client = connect(data_dir);
    let state = client.call(MethodName::GetState, None).unwrap();
    let session = state["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["id"] == json!(session_id))
        .unwrap();
    assert_eq!(
        session["status"],
        json!("stopped"),
        "the interrupted turn is surfaced as stopped, not left 'working'"
    );

    second.stop();
}

// --- bridge exec ---------------------------------------------------------------

fn run_exec(data_dir: &Path, extra: &[&str]) -> (std::process::ExitStatus, String, String) {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_bridge"))
        .arg("exec")
        .arg("--json")
        .args(["--data-dir", data_dir.to_str().unwrap()])
        .args(extra)
        .output()
        .expect("bridge exec runs");
    (
        output.status,
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn exec_attaches_to_a_running_daemon_for_a_one_shot_call() {
    let fixture = tempfile::tempdir().unwrap();
    let data_dir = fixture.path();
    seed_data_dir(data_dir);
    let running = RunningDaemon::start(data_dir);

    let (status, stdout, stderr) = run_exec(data_dir, &["--method", "health/health"]);
    assert!(status.success(), "stdout: {stdout}\nstderr: {stderr}");
    let line: serde_json::Value = serde_json::from_str(stdout.lines().next().unwrap()).unwrap();
    assert_eq!(line["type"], json!("result"));
    assert_eq!(line["result"]["ok"], json!(true));

    running.stop();
}

#[test]
fn exec_self_hosts_when_no_daemon_is_running_and_leaves_nothing_behind() {
    let fixture = tempfile::tempdir().unwrap();
    let data_dir = fixture.path();
    seed_data_dir(data_dir);

    let (status, stdout, stderr) = run_exec(
        data_dir,
        &["--method", "workspaces/create_workspace", "--params", r#"{"title":"CI"}"#],
    );
    assert!(status.success(), "stdout: {stdout}\nstderr: {stderr}");
    let line: serde_json::Value = serde_json::from_str(stdout.lines().next().unwrap()).unwrap();
    assert_eq!(line["result"]["workspaces"][0]["title"], json!("CI"));

    // One-shot means one-shot: the lease is free again (a new owner starts
    // instantly) and the state the call created is durable.
    let running = RunningDaemon::start(data_dir);
    let client = connect(data_dir);
    let state = client.call(MethodName::GetState, None).unwrap();
    assert_eq!(state["workspaces"][0]["title"], json!("CI"));
    running.stop();
}

#[test]
fn exec_reports_rpc_errors_as_json_and_a_nonzero_exit() {
    let fixture = tempfile::tempdir().unwrap();
    let data_dir = fixture.path();
    seed_data_dir(data_dir);

    let (status, stdout, _) = run_exec(
        data_dir,
        &["--method", "workspaces/create_workspace", "--params", r#"{"nope":true}"#],
    );
    assert!(!status.success());
    let line: serde_json::Value = serde_json::from_str(stdout.lines().next().unwrap()).unwrap();
    assert_eq!(line["type"], json!("error"));
    assert_eq!(line["code"], json!(-32602));
}

#[test]
fn exec_refuses_a_directory_owned_by_another_process_with_identity() {
    let fixture = tempfile::tempdir().unwrap();
    let data_dir = fixture.path();
    seed_data_dir(data_dir);
    // An embedded owner (the desktop app) holds the lease with no socket to
    // attach to: exec can neither attach nor self-host, and must say who owns
    // the directory instead of failing opaquely.
    let _embedded = bridge_core::ownership::DataDirLease::acquire(
        data_dir,
        bridge_core::ownership::OwnerKind::Embedded,
    )
    .unwrap();

    let (status, _, stderr) = run_exec(data_dir, &["--method", "health/health"]);
    assert!(!status.success());
    assert!(stderr.contains("desktop app"), "stderr: {stderr}");
    assert!(stderr.contains(&std::process::id().to_string()), "stderr: {stderr}");
}

#[test]
fn the_cursor_reflects_delivery_not_queueing() {
    let fixture = tempfile::tempdir().unwrap();
    let data_dir = fixture.path();
    seed_data_dir(data_dir);
    let running = RunningDaemon::start(data_dir);
    let client = connect(data_dir);
    let session_id = create_chat(&client);
    for text in ["one", "two", "three"] {
        publish_durable(&running.daemon, &session_id, text);
    }

    // The stream replays all three on subscribe, but has delivered nothing:
    // resuming from cursor() must not skip the queued backlog.
    let mut stream = SessionEventStream::new(&client, session_id.clone(), 0).unwrap();
    assert_eq!(stream.cursor(), 0, "nothing delivered yet");
    let first = stream.next(Instant::now() + Duration::from_secs(10)).unwrap().unwrap();
    assert_eq!(first.sequence, 1);
    assert_eq!(stream.cursor(), 1, "the cursor is the last DELIVERED sequence");

    // A consumer that reconnects from that cursor sees two and three — the
    // queued-but-undelivered events are not lost.
    let mut resumed = SessionEventStream::new(&client, session_id.clone(), stream.cursor()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let texts: Vec<String> = (0..2)
        .map(|_| {
            resumed.next(deadline).unwrap().unwrap().payload["text"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(texts, ["two", "three"]);

    running.stop();
}

#[test]
fn concurrent_session_streams_do_not_steal_each_others_events() {
    let fixture = tempfile::tempdir().unwrap();
    let data_dir = fixture.path();
    seed_data_dir(data_dir);
    let running = RunningDaemon::start(data_dir);
    let client = connect(data_dir);
    let session_a = create_chat(&client);
    let session_b = create_chat(&client);

    let mut stream_a = SessionEventStream::new(&client, session_a.clone(), 0).unwrap();
    let mut stream_b = SessionEventStream::new(&client, session_b.clone(), 0).unwrap();
    // Interleave events across both sessions on one connection.
    publish_durable(&running.daemon, &session_a, "a1");
    publish_durable(&running.daemon, &session_b, "b1");
    publish_durable(&running.daemon, &session_a, "a2");
    publish_durable(&running.daemon, &session_b, "b2");

    let deadline = Instant::now() + Duration::from_secs(15);
    let drain = |stream: &mut SessionEventStream| -> Vec<String> {
        (0..2)
            .map(|_| {
                stream.next(deadline).unwrap().expect("both streams see their events").payload
                    ["text"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect()
    };
    // Order matters: stream A drains first — with a shared consumer it would
    // have discarded B's frames while searching for its own.
    assert_eq!(drain(&mut stream_a), ["a1", "a2"]);
    assert_eq!(drain(&mut stream_b), ["b1", "b2"]);

    running.stop();
}

#[test]
fn dropped_clients_release_their_daemon_connection_slots() {
    let fixture = tempfile::tempdir().unwrap();
    let data_dir = fixture.path();
    seed_data_dir(data_dir);
    let running = RunningDaemon::start(data_dir);

    // More connect/drop cycles than the daemon has slots: if dropping a
    // client leaked its reader-side socket, the daemon would refuse long
    // before the end.
    for round in 0..(bridged::MAX_CONNECTIONS + 8) {
        let client = connect(data_dir);
        let health = client.call(MethodName::Health, None);
        assert!(health.is_ok(), "round {round}: {health:?}");
        drop(client);
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while running.daemon.state.connections.load(Ordering::SeqCst) > 0 {
        assert!(
            Instant::now() < deadline,
            "{} slot(s) still occupied after every client dropped",
            running.daemon.state.connections.load(Ordering::SeqCst)
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    running.stop();
}

#[test]
fn a_timed_out_call_does_not_desynchronize_the_next_one() {
    let fixture = tempfile::tempdir().unwrap();
    let data_dir = fixture.path();
    seed_data_dir(data_dir);
    let running = RunningDaemon::start(data_dir);
    let client = connect(data_dir);

    // A zero budget times out before the (fast) response arrives...
    let timed_out = client.call_with_timeout(MethodName::GetState, None, Duration::ZERO);
    assert!(matches!(timed_out, Err(ClientError::Timeout)), "{timed_out:?}");
    // ...and the stale response is discarded by id: the next call pairs with
    // its own response instead of the leftover one.
    let health = client.call(MethodName::Health, None).unwrap();
    assert_eq!(health["ok"], json!(true));
    let state = client.call(MethodName::GetState, None).unwrap();
    assert!(state["sessions"].is_array());

    running.stop();
}

#[test]
fn concurrent_chat_creators_receive_their_own_committed_identity() {
    let dir = tempfile::tempdir().unwrap();
    seed_data_dir(dir.path());
    let daemon = RunningDaemon::start(dir.path());
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let creators: Vec<_> = (0..8).map(|index| {
        let client = connect(dir.path());
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            let title = format!("creator {index}");
            barrier.wait();
            let result = client.call(MethodName::CreateChatId, Some(json!({
                "harness": "shell", "title": title,
            }))).unwrap();
            (result["sessionId"].as_str().unwrap().to_owned(), title)
        })
    }).collect();
    let created: Vec<_> = creators.into_iter().map(|thread| thread.join().unwrap()).collect();
    let client = connect(dir.path());
    let state = client.call(MethodName::GetState, None).unwrap();
    let sessions = state["sessions"].as_array().unwrap();
    let mut ids = std::collections::HashSet::new();
    for (id, title) in created {
        assert!(ids.insert(id.clone()), "creators must receive distinct identities");
        let session = sessions.iter().find(|session| session["id"] == id).unwrap();
        assert_eq!(session["title"], title);
    }
    drop(client);
    daemon.stop();
}

//! The desktop half of the #107 verification checklist, without a webview:
//! the proxy + supervisor drive real in-process daemons, so daemon restarts,
//! reconciliation emits, outage behavior, and error fidelity are all
//! observable exactly as the frontend would observe them.

use bridge_deck_lib::daemon_host::{supervise, DaemonProxy, Launcher};
use bridge_protocol::MethodName;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct RunningDaemon {
    daemon: Arc<bridged::Daemon>,
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
        let daemon = Arc::new(daemon);
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

/// Everything a running proxy host needs, plus the emitted-event recorder
/// standing in for the webview.
struct ProxyHost {
    proxy: Arc<DaemonProxy>,
    emitted: Arc<Mutex<Vec<(String, Value)>>>,
    stop: Arc<AtomicBool>,
    supervisor: Option<std::thread::JoinHandle<()>>,
}

impl ProxyHost {
    /// Attach-only supervision (no daemon binary): exactly the app's daemon
    /// mode against an externally managed daemon.
    fn start(data_dir: &Path) -> ProxyHost {
        let launcher = Launcher::new(
            data_dir.to_path_buf(),
            data_dir.join("no-extension"),
            None,
        );
        let proxy = Arc::new(DaemonProxy::default());
        let emitted: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let supervisor = {
            let proxy = proxy.clone();
            let emitted = emitted.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                supervise(&proxy, launcher, None, &stop, |kind, payload| {
                    emitted.lock().unwrap().push((kind.to_owned(), payload));
                });
            })
        };
        ProxyHost { proxy, emitted, stop, supervisor: Some(supervisor) }
    }

    fn wait_attached(&self, attached: bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while self.proxy.attached() != attached {
            assert!(
                Instant::now() < deadline,
                "proxy never became attached={attached}"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn emitted_kinds(&self) -> Vec<String> {
        self.emitted.lock().unwrap().iter().map(|(kind, _)| kind.clone()).collect()
    }

    fn shutdown(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.supervisor.take() {
            handle.join().unwrap();
        }
    }
}

#[test]
fn the_proxy_attaches_and_serves_calls_like_the_embedded_host() {
    let fixture = tempfile::tempdir().unwrap();
    let daemon = RunningDaemon::start(fixture.path());
    let host = ProxyHost::start(fixture.path());
    host.wait_attached(true);

    // A parameterless method and a parameterized one, both through the
    // MethodName the invoke proxy would resolve.
    let state = host.proxy.call(MethodName::GetState, None).unwrap();
    assert!(state.get("sessions").is_some(), "{state}");
    let health = host.proxy.call(MethodName::Health, None).unwrap();
    assert!(health.get("adapters").is_some(), "{health}");

    host.shutdown();
    daemon.stop();
}

#[test]
fn more_calls_than_pool_connections_keep_their_responses() {
    const CALLS: usize = 13;
    let fixture = tempfile::tempdir().unwrap();
    let daemon = RunningDaemon::start(fixture.path());
    let host = ProxyHost::start(fixture.path());
    host.wait_attached(true);

    let barrier = Arc::new(std::sync::Barrier::new(CALLS));
    let calls = (0..CALLS)
        .map(|index| {
            let proxy = host.proxy.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let (method, expected) = if index % 2 == 0 {
                    (MethodName::GetState, "sessions")
                } else {
                    (MethodName::Health, "adapters")
                };
                let response = proxy.call(method, None).unwrap();
                assert!(response.get(expected).is_some(), "{method:?}: {response}");
            })
        })
        .collect::<Vec<_>>();
    for call in calls {
        call.join().unwrap();
    }

    host.shutdown();
    daemon.stop();
}

#[test]
fn a_daemon_side_error_arrives_as_its_message_and_the_link_survives() {
    let fixture = tempfile::tempdir().unwrap();
    let daemon = RunningDaemon::start(fixture.path());
    let host = ProxyHost::start(fixture.path());
    host.wait_attached(true);

    let error = host
        .proxy
        .call(
            MethodName::GetSessionForest,
            Some(json!({"sessionId": "no-such-session"})),
        )
        .unwrap_err();
    assert!(!error.is_empty());
    // An RPC error is an answer, not a connection failure: the same link
    // keeps serving.
    assert!(host.proxy.attached());
    host.proxy.call(MethodName::GetState, None).unwrap();

    host.shutdown();
    daemon.stop();
}

#[test]
fn daemon_notifications_reach_the_webview_with_unchanged_names_and_payloads() {
    let fixture = tempfile::tempdir().unwrap();
    let daemon = RunningDaemon::start(fixture.path());
    let host = ProxyHost::start(fixture.path());
    host.wait_attached(true);

    daemon
        .daemon
        .core
        .events
        .publish(bridge_core::events::CoreEvent::SessionOutput {
            session_id: "s1".into(),
            data: "hello".into(),
        });
    daemon.daemon.core.events.publish(bridge_core::events::CoreEvent::StateChanged);

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let emitted = host.emitted.lock().unwrap().clone();
        let output = emitted.iter().find(|(kind, _)| kind == "session-output");
        if let Some((_, payload)) = output {
            assert_eq!(payload["sessionId"], "s1");
            assert_eq!(payload["data"], "hello");
            // state-changed keeps its null payload, as the webview expects.
            let state_changed = emitted
                .iter()
                .find(|(kind, _)| kind == "state-changed")
                .expect("state-changed forwarded");
            assert_eq!(state_changed.1, Value::Null);
            break;
        }
        assert!(Instant::now() < deadline, "notifications never arrived: {emitted:?}");
        std::thread::sleep(Duration::from_millis(25));
    }

    host.shutdown();
    daemon.stop();
}

#[test]
fn a_daemon_restart_reconnects_and_emits_reconciliation_hints() {
    let fixture = tempfile::tempdir().unwrap();
    let first = RunningDaemon::start(fixture.path());
    let host = ProxyHost::start(fixture.path());
    host.wait_attached(true);
    host.proxy.call(MethodName::GetState, None).unwrap();

    // Take the daemon down: the supervisor must notice and drop the link.
    first.stop();
    host.wait_attached(false);
    host.emitted.lock().unwrap().clear();

    // While no daemon is up, calls fail with a clear message instead of
    // hanging a UI thread's promise forever.
    let refused = host
        .proxy
        .call_within(MethodName::GetState, None, Duration::from_millis(300))
        .unwrap_err();
    assert!(refused.contains("not reachable"), "{refused}");

    // A new daemon on the same data directory: the supervisor reattaches on
    // its own and tells the webview to refetch what it may have missed.
    let second = RunningDaemon::start(fixture.path());
    host.wait_attached(true);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let kinds = host.emitted_kinds();
        if kinds.iter().any(|kind| kind == "state-changed")
            && kinds.iter().any(|kind| kind == "adapters-changed")
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "reconciliation hints never emitted: {kinds:?}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    host.proxy.call(MethodName::GetState, None).unwrap();

    host.shutdown();
    second.stop();
}

#[test]
fn a_pending_approval_is_recovered_after_a_daemon_restart() {
    let fixture = tempfile::tempdir().unwrap();
    let first = RunningDaemon::start(fixture.path());
    let host = ProxyHost::start(fixture.path());
    host.wait_attached(true);

    // Persist a durable approval request, as a live adapter turn would.
    let state = host.proxy.call(MethodName::GetState, None).unwrap();
    let before: std::collections::HashSet<String> = session_ids(&state);
    let state = host
        .proxy
        .call(MethodName::CreateChat, Some(json!({"harness": "shell"})))
        .unwrap();
    let session_id = session_ids(&state)
        .into_iter()
        .find(|id| !before.contains(id))
        .expect("chat created");
    {
        let db = first.daemon.core.db.lock().unwrap();
        bridge_core::store::session_event(
            &db,
            &session_id,
            &bridge_core::agent::NormalizedEvent {
                kind: "approval.requested".into(),
                item_id: None,
                role: None,
                status: Some("pending".into()),
                title: Some("Run tests?".into()),
                text: None,
                data: json!({"requestId": 7}),
            },
            &json!({"adapter": "test"}),
        )
        .unwrap();
    }

    first.stop();
    let second = RunningDaemon::start(fixture.path());
    host.wait_attached(true);

    // The approval is durable state, not a live-channel artifact: replaying
    // the session after the restart surfaces it, so the webview's refetch
    // (triggered by the reconciliation hints) recovers the pending card.
    let events = host
        .proxy
        .call(
            MethodName::ReplaySessionEvents,
            Some(json!({"sessionId": session_id, "afterSequence": 0})),
        )
        .unwrap();
    let replayed = events.as_array().unwrap();
    assert!(
        replayed.iter().any(|event| event["kind"] == "approval.requested"),
        "approval lost across restart: {replayed:?}"
    );

    host.shutdown();
    second.stop();
}

fn session_ids(state: &Value) -> std::collections::HashSet<String> {
    state["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|session| session["id"].as_str().unwrap().to_owned())
        .collect()
}

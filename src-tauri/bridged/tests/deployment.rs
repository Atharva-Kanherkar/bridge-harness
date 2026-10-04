//! The deployment a daemon is pinned to is reported on `health/health` and
//! enforced at harness start. One test per process: the deployment is
//! process-wide state.

use bridge_core::credential_policy::{self, CredentialPolicy, DeploymentInfo, ExecutionTopology};
use bridged::{Daemon, DaemonConfig};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

#[test]
fn a_pinned_deployment_is_reported_and_enforced_at_harness_start() {
    credential_policy::configure(DeploymentInfo {
        topology: ExecutionTopology::RemoteRunner,
        credential_policy: CredentialPolicy::ApiKeyOnly,
    });
    let fixture = tempfile::tempdir().unwrap();
    let (daemon, listener) = Daemon::start(DaemonConfig {
        data_dir: fixture.path().to_path_buf(),
        socket_path: None,
        health_addr: None,
        browser_extension_path: fixture.path().join("no-extension"),
        handshake_timeout: Duration::from_millis(400),
    })
    .expect("daemon starts");
    let daemon = Arc::new(daemon);
    let token = daemon.state.auth_token.clone();
    let socket_path = daemon.socket_path.clone();
    let serving = {
        let daemon = daemon.clone();
        std::thread::spawn(move || bridged::serve(&daemon, listener).unwrap())
    };

    let stream = UnixStream::connect(&socket_path).unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut writer = stream;
    // Notifications (boot-time discovery events) interleave with responses,
    // so a call returns the frame that answers it, not merely the next one.
    let mut call = |frame: Value| -> Value {
        let id = frame["id"].clone();
        writeln!(writer, "{frame}").unwrap();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let received: Value = serde_json::from_str(&line).unwrap();
            if received.get("id") == Some(&id) {
                return received;
            }
        }
    };
    let handshake = call(json!({
        "jsonrpc": "2.0", "id": 0, "method": "protocol/handshake",
        "params": {
            "protocolVersion": {
                "major": bridge_protocol::PROTOCOL_VERSION.major,
                "minor": bridge_protocol::PROTOCOL_VERSION.minor,
            },
            "client": {"name": "deployment-test", "version": "0"},
            "authToken": token,
        },
    }));
    assert!(handshake.get("error").is_none());

    let health = call(json!({"jsonrpc": "2.0", "id": 1, "method": "health/health"}));
    assert_eq!(
        health["result"]["deployment"],
        json!({"topology": "remote-runner", "credentialPolicy": "api-key-only"})
    );

    // The adapter gate refuses a harness whose credential Bridge cannot
    // verify, with the stable code, before any process is spawned.
    let error = daemon
        .core
        .adapter_registry
        .start(
            "opencode",
            bridge_core::adapters::StartRequest {
                cwd: "/tmp",
                model: None,
                effort: None,
                instructions: None,
                write_mode: None,
                read_only_sandbox: None,
                briefing: None,
                on_progress: None,
            },
        )
        .err()
        .expect("opencode is refused under api-key-only");
    assert_eq!(
        bridge_protocol::ErrorCode::from(&error),
        bridge_protocol::ErrorCode::CredentialPolicyViolation
    );
    assert!(error.to_string().contains("api-key-only"), "{error}");

    daemon.state.shutting_down.store(true, Ordering::SeqCst);
    serving.join().unwrap();
    daemon.shutdown(Duration::from_secs(5));
}

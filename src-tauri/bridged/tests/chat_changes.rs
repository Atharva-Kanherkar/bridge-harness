//! Chat-scoped changes over the socket (testing/feat-chat-scoped-changes.md):
//! a chat sees only what its own turns changed, a chat with no folder says
//! why it has no scope, and bad params are client errors.

// A test harness: a failed unwrap or expect in a helper is the test failing,
// which is the behavior wanted. Clippy's in-tests allowances cover `#[test]`
// fns but not this file's helpers.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use bridge_core::chat_changes::{record, Phase};
use bridged::{Daemon, DaemonConfig};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

fn git(cwd: &Path, args: &[&str]) {
    let output = std::process::Command::new("git").args(args).current_dir(cwd).output().unwrap();
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
}

fn seed(data_dir: &Path, repo: &Path) {
    let db = bridge_core::store::open(&data_dir.join("bridge.db")).unwrap();
    db.execute(
        "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo',?1,'now')",
        [repo.to_string_lossy()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','c','t','main',?1,'ready','now')",
        [repo.to_string_lossy()],
    )
    .unwrap();
    db.execute_batch(
        "INSERT INTO sessions(id,workspace_id,harness,label,status,kind) VALUES('chat','w','claude','Chat','ready','direct');
         INSERT INTO sessions(id,workspace_id,harness,label,status,kind) VALUES('loose',NULL,'claude','Loose','ready','direct');",
    )
    .unwrap();
}

struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next: i64,
}

impl Client {
    fn connect(socket_path: &Path, token: &str) -> Client {
        let stream = UnixStream::connect(socket_path).expect("client connects");
        stream.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        let mut client = Client { reader: BufReader::new(stream.try_clone().unwrap()), writer: stream, next: 0 };
        let handshake = client.call(
            "protocol/handshake",
            json!({
                "protocolVersion": {
                    "major": bridge_protocol::PROTOCOL_VERSION.major,
                    "minor": bridge_protocol::PROTOCOL_VERSION.minor,
                },
                "client": {"name": "chat-changes-test", "version": "0"},
                "authToken": token,
            }),
        );
        assert!(handshake.get("result").is_some(), "handshake failed: {handshake}");
        client
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        let mut line =
            serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).unwrap();
        line.push(b'\n');
        self.writer.write_all(&line).unwrap();
        self.writer.flush().unwrap();
        loop {
            let mut frame = String::new();
            self.reader.read_line(&mut frame).expect("a frame arrives");
            assert!(!frame.is_empty(), "the daemon closed the connection");
            let frame: Value = serde_json::from_str(&frame).unwrap();
            if frame.get("id") == Some(&json!(id)) {
                return frame;
            }
        }
    }
}


#[test]
fn a_chat_sees_only_its_own_turns_over_the_socket() {
    let fixture = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    git(repo.path(), &["init", "-q"]);
    git(repo.path(), &["config", "user.email", "t@example.invalid"]);
    git(repo.path(), &["config", "user.name", "T"]);
    git(repo.path(), &["config", "commit.gpgsign", "false"]);
    std::fs::write(repo.path().join("README.md"), "seed\n").unwrap();
    git(repo.path(), &["add", "."]);
    git(repo.path(), &["commit", "-qm", "seed"]);
    std::fs::write(repo.path().join("leftover.txt"), "old work\n").unwrap();
    seed(fixture.path(), repo.path());
    let (daemon, listener) = Daemon::start(DaemonConfig {
        data_dir: fixture.path().to_path_buf(),
        socket_path: None,
        health_addr: None,
        browser_extension_path: fixture.path().join("no-extension"),
        handshake_timeout: Duration::from_millis(400),
    })
    .expect("daemon starts");
    let daemon = std::sync::Arc::new(daemon);
    let serve_daemon = daemon.clone();
    std::thread::spawn(move || {
        bridged::serve(&serve_daemon, listener).expect("accept loop serves");
    });
    let mut client = Client::connect(&daemon.socket_path, &daemon.state.auth_token);

    let before = client.call("workspaces/session_changes", json!({"sessionId": "chat"}));
    assert_eq!(before["result"]["available"], json!(false), "{before}");
    assert_eq!(before["result"]["unavailableReason"], json!("no_turns"), "{before}");

    record(repo.path(), "chat", Phase::Start, Some("t1")).unwrap();
    std::fs::write(repo.path().join("README.md"), "seed\nedited\n").unwrap();
    record(repo.path(), "chat", Phase::End, Some("t1")).unwrap();

    let stats = client.call("workspaces/session_changes", json!({"sessionId": "chat"}));
    assert_eq!(stats["result"]["available"], json!(true), "{stats}");
    assert_eq!(stats["result"]["turns"], json!(1));
    let files = stats["result"]["changes"]["files"].as_array().unwrap();
    assert_eq!(files.len(), 1, "{stats}");
    assert_eq!(files[0]["path"], json!("README.md"));
    assert_eq!(files[0]["additions"], json!(1));
    assert_eq!(files[0]["patch"], json!(""));

    let full = client.call("workspaces/session_changes", json!({"sessionId": "chat", "includePatches": true}));
    assert!(full["result"]["changes"]["files"][0]["patch"].as_str().unwrap().contains("+edited"), "{full}");

    let loose = client.call("workspaces/session_changes", json!({"sessionId": "loose"}));
    assert_eq!(loose["result"]["unavailableReason"], json!("no_folder"), "{loose}");

    let unknown = client.call("workspaces/session_changes", json!({"sessionId": "chat", "bogus": 1}));
    assert_eq!(unknown["error"]["code"], json!(-32602), "{unknown}");
}

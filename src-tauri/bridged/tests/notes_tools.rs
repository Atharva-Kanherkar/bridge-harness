//! The notes tools against a real daemon: a chat's helper reaches the URL it
//! was launched with and can write notes. Its own test binary, because the
//! notes route serves whichever core the process installed.

use bridged::{Daemon, DaemonConfig};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Duration;

fn seed(data_dir: &Path) {
    let db = bridge_core::store::open(&data_dir.join("bridge.db")).unwrap();
    let mut opencode = bridge_core::agent_config::state(&db)
        .unwrap()
        .harnesses
        .into_iter()
        .find(|harness| harness.id == "opencode")
        .unwrap();
    opencode.advanced = json!({"executablePath": data_dir.join("missing-opencode").to_string_lossy()});
    bridge_core::agent_config::save_harness(&db, opencode).unwrap();
    db.execute_batch(
        "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/notes-tools-demo','now');
         INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','c','t','main','/tmp/notes-tools-demo/w','ready','now');
         INSERT INTO sessions(id,workspace_id,harness,label,status) VALUES('chat-1','w','claude','chat','ready');",
    )
    .unwrap();
}

/// One HTTP request to the URL a helper was given; returns status and JSON body.
fn request(url: &str, method: &str, token: &str, body: &Value) -> (u16, Value) {
    let rest = url.strip_prefix("http://").expect("an http URL");
    let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let mut stream = match TcpStream::connect(authority) {
        Ok(stream) => stream,
        Err(error) => return (0, json!({"error": error.to_string()})),
    };
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let payload = body.to_string();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {authority}\r\nx-bridge-notes-auth: {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(payload.as_bytes()).unwrap();
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    let status = response.split_whitespace().nth(1).and_then(|code| code.parse().ok()).unwrap_or(0);
    let body = response.split("\r\n\r\n").nth(1).and_then(|text| serde_json::from_str(text).ok()).unwrap_or(Value::Null);
    (status, body)
}

#[test]
fn the_daemon_serves_notes_tools_on_its_own_listener() {
    let fixture = tempfile::tempdir().unwrap();
    seed(fixture.path());
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
    let accept_loop = std::thread::spawn(move || bridged::serve(&serve_daemon, listener).expect("accept loop serves"));

    // Exactly what a chat's tool server receives at launch.
    let env = bridge_core::notes::tool::launch_env("chat-1");
    let url = env.iter().find(|(key, _)| key == bridge_core::notes::tool::ENV_URL).map(|(_, value)| value.clone()).expect("a notes URL");
    let token = env.iter().find(|(key, _)| key == bridge_core::notes::tool::ENV_TOKEN).map(|(_, value)| value.clone()).expect("a notes token");

    let (status, tools) = request(&format!("{url}/tools"), "GET", &token, &json!({}));
    assert_eq!(status, 200, "the daemon must answer the URL helpers are given: {tools}");
    assert_eq!(tools["access"], "write");
    let (status, created) = request(&format!("{url}/notes_create"), "POST", &token, &json!({"title": "From the daemon", "content": "written by a chat"}));
    assert_eq!(status, 200, "{created}");
    assert_eq!(created["created"]["lastEditedBy"], "agent:claude:chat-1");
    let (status, _) = request(&format!("{url}/notes_create"), "POST", "wrong", &json!({"title": "x"}));
    assert_eq!(status, 401);
    let page = daemon.core.notes.list(&Default::default()).unwrap();
    assert_eq!(page.len(), 1);

    daemon.state.shutting_down.store(true, Ordering::SeqCst);
    accept_loop.join().unwrap();
    daemon.shutdown(Duration::from_secs(5));
}

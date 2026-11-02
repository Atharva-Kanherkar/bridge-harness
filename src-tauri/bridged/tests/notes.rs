//! `notes/*` over the daemon socket: capture, read, a merged save, update,
//! list and delete, with a `notes/page_changed` notification per write.

use bridged::{Daemon, DaemonConfig};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Duration;

fn seed(data_dir: &Path) {
    let db = bridge_core::store::open(&data_dir.join("bridge.db")).unwrap();
    // OpenCode discovery must fail fast rather than spawn a server.
    let mut opencode = bridge_core::agent_config::state(&db)
        .unwrap()
        .harnesses
        .into_iter()
        .find(|harness| harness.id == "opencode")
        .unwrap();
    opencode.advanced = json!({"executablePath": data_dir.join("missing-opencode").to_string_lossy()});
    bridge_core::agent_config::save_harness(&db, opencode).unwrap();
}

struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    notifications: Vec<Value>,
}

impl Client {
    fn connect(socket_path: &Path, token: &str) -> Client {
        let stream = UnixStream::connect(socket_path).expect("client connects");
        stream.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        let mut client = Client { reader: BufReader::new(stream.try_clone().unwrap()), writer: stream, notifications: Vec::new() };
        let handshake = client.call(
            0,
            "protocol/handshake",
            json!({
                "protocolVersion": {"major": bridge_protocol::PROTOCOL_VERSION.major, "minor": bridge_protocol::PROTOCOL_VERSION.minor},
                "client": {"name": "notes-test", "version": "0"},
                "authToken": token,
            }),
        );
        assert!(handshake.get("result").is_some(), "handshake failed: {handshake}");
        client
    }

    fn read_frame(&mut self) -> Option<Value> {
        let mut frame = String::new();
        match self.reader.read_line(&mut frame) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(serde_json::from_str(&frame).unwrap()),
        }
    }

    fn call(&mut self, id: i64, method: &str, params: Value) -> Value {
        let mut line = serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).unwrap();
        line.push(b'\n');
        self.writer.write_all(&line).unwrap();
        self.writer.flush().unwrap();
        loop {
            let frame = self.read_frame().expect("a frame arrives");
            if frame.get("id").is_some() && frame.get("method").is_none() {
                assert_eq!(frame["id"], json!(id));
                return frame;
            }
            self.notifications.push(frame);
        }
    }

    /// Page changes seen so far, waiting briefly for any still in flight.
    fn page_changes(&mut self, expected: usize) -> Vec<Value> {
        self.writer.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
        let changes = |notifications: &[Value]| notifications.iter().filter(|frame| frame["method"] == json!("notes/page_changed")).cloned().collect::<Vec<_>>();
        while changes(&self.notifications).len() < expected {
            match self.read_frame() {
                Some(frame) => self.notifications.push(frame),
                None => break,
            }
        }
        changes(&self.notifications)
    }
}

#[test]
fn notes_methods_round_trip_through_the_daemon() {
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
    let mut client = Client::connect(&daemon.socket_path, &daemon.state.auth_token);

    let typed = "helo, two things: buld the notes app\nand call the bank";
    let captured = client.call(1, "notes/capture_note", json!({"body": typed}));
    assert!(captured.get("error").is_none(), "{captured}");
    let id = captured["result"]["id"].as_str().unwrap().to_owned();
    assert_eq!(captured["result"]["state"], json!("inbox"));
    assert!(captured["result"]["workspaceId"].is_null());

    let page = client.call(2, "notes/read_page", json!({"id": id}));
    assert_eq!(page["result"]["markdown"], json!(typed));
    let base_seq = page["result"]["seq"].as_i64().unwrap();

    // An agent-style append lands first; the person's older save still merges.
    let appended = client.call(3, "notes/edit_page", json!({"id": id, "append": {"content": "- and a third thing"}}));
    assert!(appended.get("error").is_none(), "{appended}");
    let saved = client.call(4, "notes/edit_page", json!({"id": id, "baseSeq": base_seq, "text": typed.replace("helo", "hello")}));
    assert!(saved.get("error").is_none(), "{saved}");
    let merged = client.call(5, "notes/read_page", json!({"id": id}));
    assert_eq!(merged["result"]["markdown"], json!("hello, two things: buld the notes app\nand call the bank\n- and a third thing"));

    let updated = client.call(6, "notes/update_note", json!({"id": id, "title": "Two things", "tags": ["ideas"], "addSessionId": "s1"}));
    assert_eq!(updated["result"]["title"], json!("Two things"));
    assert_eq!(updated["result"]["sessionIds"], json!(["s1"]));
    let unknown = client.call(7, "notes/update_note", json!({"id": id, "workspaceId": "no-such-project"}));
    assert!(unknown.get("error").is_some(), "an unknown project is refused: {unknown}");
    let strict = client.call(8, "notes/capture_note", json!({"body": "x", "extra": true}));
    assert!(strict.get("error").is_some(), "unknown params are refused: {strict}");

    let listed = client.call(9, "notes/list_notes", json!({"query": "bank"}));
    assert_eq!(listed["result"]["notes"].as_array().unwrap().len(), 1);
    assert!(listed["result"]["root"].as_str().unwrap().ends_with("notes"));

    // History lists the four changes so far; the capture's words come back.
    let history = client.call(20, "notes/page_history", json!({"id": id}));
    let versions = history["result"]["versions"].as_array().unwrap_or_else(|| panic!("{history}")).clone();
    assert_eq!(versions.len(), 4, "{history}");
    let first = versions.last().unwrap()["seq"].as_i64().unwrap();
    let version = client.call(21, "notes/read_version", json!({"id": id, "seq": first}));
    assert_eq!(version["result"]["markdown"], json!(typed));
    let restored = client.call(22, "notes/restore_version", json!({"id": id, "seq": first}));
    assert!(restored.get("error").is_none(), "{restored}");
    let page = client.call(23, "notes/read_page", json!({"id": id}));
    assert_eq!(page["result"]["markdown"], json!(typed));
    let strict = client.call(24, "notes/restore_version", json!({"id": id, "seq": first, "extra": 1}));
    assert!(strict.get("error").is_some(), "{strict}");

    // An image in two parts ("PNG-" then "bytes"), stored and read back.
    let part = client.call(30, "notes/put_asset_part", json!({"data": "UE5HLQ=="}));
    let upload = part["result"]["uploadId"].as_str().unwrap_or_else(|| panic!("{part}")).to_owned();
    let part = client.call(31, "notes/put_asset_part", json!({"uploadId": upload, "data": "Ynl0ZXM="}));
    assert_eq!(part["result"]["size"], json!(9), "{part}");
    let asset = client.call(32, "notes/finish_asset", json!({"uploadId": upload, "mime": "image/png"}));
    let path = asset["result"]["path"].as_str().unwrap_or_else(|| panic!("{asset}")).to_owned();
    assert!(path.starts_with("assets/") && path.ends_with(".png"), "{asset}");
    let read = client.call(33, "notes/read_asset", json!({"path": path}));
    assert_eq!(read["result"]["data"], json!("UE5HLWJ5dGVz"), "{read}");
    assert_eq!(read["result"]["mime"], json!("image/png"));
    let refused = client.call(34, "notes/read_asset", json!({"path": "assets/../notes.db"}));
    assert!(refused.get("error").is_some(), "{refused}");

    // `cursor` cannot run note passes, so the daemon's live loop settles this
    // job as failed without ever starting a model turn on the test machine.
    let queued = client.call(10, "notes/structure_note", json!({"id": id, "harness": "cursor"}));
    assert!(queued["result"]["jobId"].is_string(), "{queued}");
    let settings = client.call(11, "notes/get_notes_settings", json!(null));
    assert!(settings.get("error").is_some(), "a parameterless method refuses params: {settings}");
    let settings = client.call(12, "notes/save_notes_settings", json!({}));
    assert_eq!(settings["result"]["autoStructure"], json!(false));

    let deleted = client.call(13, "notes/delete_note", json!({"id": id}));
    assert!(deleted.get("error").is_none(), "{deleted}");
    let gone = client.call(14, "notes/read_page", json!({"id": id}));
    assert!(gone.get("error").is_some());

    // capture, append, save, update, restore, delete: one change notification each.
    let changes = client.page_changes(6);
    let actors: Vec<&str> = changes.iter().map(|frame| frame["params"]["actor"].as_str().unwrap_or("")).collect();
    assert_eq!(actors, vec!["person"; 6], "{changes:?}");
    assert!(changes.iter().all(|frame| frame["params"]["pageId"] == json!(id)));
    assert!(fixture.path().join("notes.db").exists());

    daemon.state.shutting_down.store(true, Ordering::SeqCst);
    accept_loop.join().unwrap();
    daemon.shutdown(Duration::from_secs(5));
}

#[test]
fn links_search_rename_and_parent_moves_round_trip_through_the_daemon() {
    let fixture = tempfile::tempdir().unwrap();
    seed(fixture.path());
    let (daemon, listener) = Daemon::start(DaemonConfig {
        data_dir: fixture.path().to_path_buf(), socket_path: None, health_addr: None,
        browser_extension_path: fixture.path().join("no-extension"), handshake_timeout: Duration::from_millis(400),
    }).unwrap();
    let daemon = std::sync::Arc::new(daemon);
    let server = daemon.clone();
    let accept_loop = std::thread::spawn(move || bridged::serve(&server, listener).unwrap());
    let mut client = Client::connect(&daemon.socket_path, &daemon.state.auth_token);
    let source = client.call(1, "notes/create_page", json!({"title": "Source", "content": "[[Launch Plan|the plan]]"}));
    let source = source["result"]["id"].as_str().unwrap().to_owned();
    let target = client.call(2, "notes/create_page", json!({"title": "Launch Plan", "content": "running"}));
    let target = target["result"]["id"].as_str().unwrap().to_owned();
    let read = client.call(3, "notes/read_page", json!({"id": target}));
    assert_eq!(read["result"]["backlinks"][0]["pageId"], source);
    assert_eq!(read["result"]["backlinks"][0]["excerpt"], "the plan");
    let rename = client.call(4, "notes/update_note", json!({"id": target, "title": "Q4 Launch Plan"}));
    assert!(rename.get("error").is_none(), "{rename}");
    let read = client.call(5, "notes/read_page", json!({"id": source}));
    assert_eq!(read["result"]["markdown"], "[[Q4 Launch Plan|the plan]]");
    let history = client.call(6, "notes/page_history", json!({"id": source}));
    assert_eq!(history["result"]["versions"].as_array().unwrap().len(), 2);
    let moved = client.call(7, "notes/update_note", json!({"id": source, "parentId": target}));
    assert_eq!(moved["result"]["parentId"], target);
    let cycle = client.call(8, "notes/update_note", json!({"id": target, "parentId": source}));
    assert!(cycle.get("error").is_some(), "{cycle}");
    let cleared = client.call(9, "notes/update_note", json!({"id": source, "clearParent": true}));
    assert!(cleared["result"]["parentId"].is_null());
    let search = client.call(10, "notes/list_notes", json!({"query": "runs"}));
    assert_eq!(search["result"]["notes"][0]["id"], target);
    daemon.state.shutting_down.store(true, Ordering::SeqCst);
    accept_loop.join().unwrap();
    daemon.shutdown(Duration::from_secs(5));
}

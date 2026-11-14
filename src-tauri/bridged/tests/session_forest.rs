//! `sessions/get_session_forest` over the socket: a long chat opens on a
//! small first page, then its full window, and an old client that sends no
//! `limit` still gets the full window.

use bridged::{Daemon, DaemonConfig};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

const ENTRIES: usize = 2_000;

fn seed(data_dir: &Path) {
    let db = bridge_core::store::open(&data_dir.join("bridge.db")).unwrap();
    db.execute(
        "INSERT INTO sessions(id,harness,label,status,metric_source) VALUES('long','claude','Long chat','ready','reported')",
        [],
    )
    .unwrap();
    let mut parent: Option<String> = None;
    for index in 0..ENTRIES {
        let entry = bridge_core::store::append_session_entry(
            &db,
            "long",
            parent.as_deref(),
            if index % 2 == 0 { "user.message" } else { "assistant.message" },
            &json!({"text": format!("message {index}")}),
            None,
            "eligible",
            None,
        )
        .unwrap();
        parent = Some(entry.id);
    }
}

struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next_id: i64,
}

impl Client {
    fn connect(socket_path: &Path, token: &str) -> Client {
        let stream = UnixStream::connect(socket_path).expect("client connects");
        stream.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        let mut client = Client { reader: BufReader::new(stream.try_clone().unwrap()), writer: stream, next_id: 0 };
        let handshake = client.call(
            "protocol/handshake",
            json!({
                "protocolVersion": {
                    "major": bridge_protocol::PROTOCOL_VERSION.major,
                    "minor": bridge_protocol::PROTOCOL_VERSION.minor,
                },
                "client": {"name": "forest-test", "version": "0"},
                "authToken": token,
            }),
        );
        assert!(handshake.get("result").is_some(), "handshake failed: {handshake}");
        client
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
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

    fn forest(&mut self, params: Value) -> Value {
        let reply = self.call("sessions/get_session_forest", params);
        assert!(reply.get("error").is_none(), "{reply}");
        reply["result"].clone()
    }
}

fn sequences(forest: &Value) -> Vec<i64> {
    forest["entries"].as_array().unwrap().iter().map(|entry| entry["sequence"].as_i64().unwrap()).collect()
}

#[test]
fn a_long_chat_pages_its_newest_entries_and_still_serves_the_full_window() {
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
    let accept_loop = std::thread::spawn(move || {
        bridged::serve(&serve_daemon, listener).expect("accept loop serves");
    });
    let mut client = Client::connect(&daemon.socket_path, &daemon.state.auth_token);
    let newest = ENTRIES as i64;

    // The first page: the newest 160, oldest first, with the whole branch's length.
    let page = client.forest(json!({"sessionId": "long", "limit": 160}));
    assert_eq!(sequences(&page), ((newest - 159)..=newest).collect::<Vec<_>>());
    assert_eq!(page["entryWindow"], json!({"returned": 160, "total": ENTRIES, "trimmedPayloads": 0}));
    assert_eq!(page["leaves"].as_array().unwrap().len(), 1);

    // A client that predates `limit` gets the full window, as before.
    let full = client.forest(json!({"sessionId": "long"}));
    assert_eq!(sequences(&full), ((newest - 1499)..=newest).collect::<Vec<_>>());
    assert_eq!(full["entryWindow"]["returned"], json!(1500));
    assert_eq!(full["entryWindow"]["total"], json!(ENTRIES));
    // The page is exactly the full window's tail.
    assert_eq!(page["entries"].as_array().unwrap()[..], full["entries"].as_array().unwrap()[1340..]);

    // Out-of-range limits clamp instead of failing.
    assert_eq!(client.forest(json!({"sessionId": "long", "limit": 0}))["entryWindow"]["returned"], json!(1));
    assert_eq!(client.forest(json!({"sessionId": "long", "limit": 50_000}))["entryWindow"]["returned"], json!(1500));

    // Reading never moves the digest: a reopen can trust a cached snapshot.
    let digest = client.call("sessions/get_session_forest_digest", json!({"sessionId": "long"}));
    let again = client.call("sessions/get_session_forest_digest", json!({"sessionId": "long"}));
    assert_eq!(digest["result"], again["result"]);

    drop(client);
    daemon.state.shutting_down.store(true, std::sync::atomic::Ordering::SeqCst);
    accept_loop.join().unwrap();
    daemon.shutdown(Duration::from_secs(5));
}

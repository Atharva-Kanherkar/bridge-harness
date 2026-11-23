//! Screen Time over the socket (testing/feat-usage-screen-time.md): the
//! webview's minutes land once, agent history written before migration 88 is
//! counted by the background backfill without blocking the reply, and bad
//! params are client errors.

// A test harness: a failed unwrap or expect in a helper is the test failing,
// which is the behavior wanted. Clippy's in-tests allowances cover `#[test]`
// fns but not this file's helpers.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use bridged::{Daemon, DaemonConfig};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

/// A chat with entries at 10:00, 10:01 and 10:03 (one silent minute, filled)
/// and 10:20, a briefing that never counts, and a message of the person's.
/// The minute rows are then wiped and the cursor rewound, which is exactly
/// what a history from before migration 88 looks like.
fn seed(data_dir: &Path) {
    let db = bridge_core::store::open(&data_dir.join("bridge.db")).unwrap();
    db.execute_batch(
        "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/screen-time-demo','now');
         INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','c','t','main','/tmp/screen-time-demo/w','ready','now');
         INSERT INTO sessions(id,workspace_id,harness,label,status,kind) VALUES('chat','w','claude','Chat','ready','direct');
         INSERT INTO sessions(id,workspace_id,harness,label,status,kind) VALUES('brief','w','claude','Briefing','ready','briefing');
         INSERT INTO session_entries(id,session_id,sequence,kind,created_at) VALUES
             ('e1','chat',1,'user.message','2026-03-01T09:58:00Z'),
             ('e2','chat',2,'assistant.message','2026-03-01T10:00:05Z'),
             ('e3','chat',3,'tool.completed','2026-03-01T10:01:40.123456+00:00'),
             ('e4','chat',4,'assistant.message','2026-03-01T10:03:00Z'),
             ('e5','chat',5,'assistant.message','2026-03-01T10:20:00Z'),
             ('e6','brief',1,'assistant.message','2026-03-01T10:00:00Z');
         DELETE FROM agent_activity_minutes;
         UPDATE screen_time_backfill SET cursor = 0, end_rowid = (SELECT MAX(rowid) FROM session_entries);",
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
                "client": {"name": "screen-time-test", "version": "0"},
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
fn screen_time_records_presence_and_backfills_agent_history_over_the_socket() {
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
    std::thread::spawn(move || {
        bridged::serve(&serve_daemon, listener).expect("accept loop serves");
    });
    let mut client = Client::connect(&daemon.socket_path, &daemon.state.auth_token);

    // Your time: minutes near now land once; a repeat stores nothing.
    let now_minute = chrono::Utc::now().timestamp() / 60;
    let recorded = client.call("usage/record_presence", json!({"minutes": [now_minute - 1, now_minute, now_minute + 5]}));
    assert_eq!(recorded["result"], json!({"recorded": 2}), "{recorded}");
    let again = client.call("usage/record_presence", json!({"minutes": [now_minute]}));
    assert_eq!(again["result"], json!({"recorded": 0}), "{again}");

    // The first ask answers at once and says history is still being counted.
    let first = client.call("usage/screen_time", json!({"sinceDay": "2026-03-01", "untilDay": "2026-03-01", "timeZone": "UTC"}));
    assert!(first.get("error").is_none(), "{first}");
    assert_eq!(first["result"]["allTime"]["humanSeconds"], json!(120));
    let first_status = first["result"]["agentHistory"].clone();
    assert!(first_status == json!("counting") || first_status == json!("complete"), "{first}");

    let deadline = Instant::now() + Duration::from_secs(20);
    let done = loop {
        let reply = client.call("usage/screen_time", json!({"sinceDay": "2026-03-01", "untilDay": "2026-03-01", "timeZone": "UTC"}));
        if reply["result"]["agentHistory"] == json!("complete") {
            break reply;
        }
        assert!(Instant::now() < deadline, "the backfill never finished: {reply}");
        std::thread::sleep(Duration::from_millis(100));
    };
    // 10:00, 10:01, 10:02 (filled), 10:03 and 10:20; the briefing and the
    // person's own message never count.
    assert_eq!(done["result"]["days"], json!([{"day": "2026-03-01", "humanSeconds": 0, "agentSeconds": 300}]));
    assert_eq!(done["result"]["allTime"]["agentSeconds"], json!(300));
    assert_eq!(done["result"]["allTime"]["agentSinceDay"], json!("2026-03-01"));
    assert_eq!(done["result"]["timeZone"], json!("UTC"));

    // A new entry after the migration is counted by the trigger.
    {
        let db = daemon.core.db.lock().unwrap();
        db.execute(
            "INSERT INTO session_entries(id,session_id,sequence,kind,created_at) VALUES('e7','chat',6,'assistant.message','2026-03-01T10:21:00Z')",
            [],
        )
        .unwrap();
    }
    let later = client.call("usage/screen_time", json!({"sinceDay": "2026-03-01", "untilDay": "2026-03-01"}));
    assert_eq!(later["result"]["allTime"]["agentSeconds"], json!(360), "{later}");

    // Client errors, not crashes.
    let unknown = client.call("usage/screen_time", json!({"sinceDay": "2026-03-01", "untilDay": "2026-03-01", "bogus": 1}));
    assert_eq!(unknown["error"]["code"], json!(-32602), "{unknown}");
    let too_long = client.call("usage/screen_time", json!({"sinceDay": "2026-01-01", "untilDay": "2026-06-01"}));
    assert!(too_long["error"]["message"].as_str().unwrap_or_default().contains("92 days"), "{too_long}");
    let flood: Vec<i64> = (0..1_441).map(|offset| now_minute - offset).collect();
    let refused = client.call("usage/record_presence", json!({"minutes": flood}));
    assert!(refused["error"]["message"].as_str().unwrap_or_default().contains("1440"), "{refused}");
}

//! `usage/scan_history` and `usage/list_history_sources` over the socket
//! against a temporary home holding one synthetic Claude transcript line.

use bridged::{Daemon, DaemonConfig};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Duration;

fn seed_daemon_dir(data_dir: &Path) {
    let db = bridge_core::store::open(&data_dir.join("bridge.db")).unwrap();
    let mut opencode = bridge_core::agent_config::state(&db)
        .unwrap()
        .harnesses
        .into_iter()
        .find(|harness| harness.id == "opencode")
        .unwrap();
    opencode.advanced = json!({
        "executablePath": data_dir.join("missing-opencode").to_string_lossy(),
    });
    bridge_core::agent_config::save_harness(&db, opencode).unwrap();
}

fn seed_home(home: &Path) {
    let project = home.join(".claude").join("projects").join("-Users-me-proj");
    std::fs::create_dir_all(&project).unwrap();
    let line = concat!(
        r#"{"parentUuid":"p","isSidechain":false,"cwd":"/Users/me/proj","sessionId":"sess-1","version":"2.1.261","type":"assistant","uuid":"u-1","timestamp":"2026-03-01T12:00:00.000Z","requestId":"req-1","#,
        r#""message":{"id":"msg-1","type":"message","role":"assistant","model":"claude-opus-4-6","content":[{"type":"text","text":"SECRET PROMPT TEXT"}],"usage":{"input_tokens":12,"cache_creation_input_tokens":3,"cache_read_input_tokens":40,"output_tokens":9}}}"#,
        "\n"
    );
    std::fs::write(project.join("sess-1.jsonl"), line).unwrap();
}

struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Client {
    fn connect(socket_path: &Path, token: &str) -> Client {
        let stream = UnixStream::connect(socket_path).expect("client connects");
        stream.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        let mut client = Client { reader: BufReader::new(stream.try_clone().unwrap()), writer: stream };
        let handshake = client.call(
            0,
            "protocol/handshake",
            Some(json!({
                "protocolVersion": {
                    "major": bridge_protocol::PROTOCOL_VERSION.major,
                    "minor": bridge_protocol::PROTOCOL_VERSION.minor,
                },
                "client": {"name": "usage-history-test", "version": "0"},
                "authToken": token,
            })),
        );
        assert!(handshake.get("result").is_some(), "handshake failed: {handshake}");
        client
    }

    fn call(&mut self, id: i64, method: &str, params: Option<Value>) -> Value {
        let mut frame = json!({"jsonrpc": "2.0", "id": id, "method": method});
        if let Some(params) = params {
            frame["params"] = params;
        }
        let mut line = serde_json::to_vec(&frame).unwrap();
        line.push(b'\n');
        self.writer.write_all(&line).unwrap();
        self.writer.flush().unwrap();
        loop {
            let mut text = String::new();
            self.reader.read_line(&mut text).expect("a frame arrives");
            assert!(!text.is_empty(), "the daemon closed the connection");
            let frame: Value = serde_json::from_str(&text).unwrap();
            if frame.get("id").is_some() {
                assert_eq!(frame["id"], json!(id));
                return frame;
            }
        }
    }
}

#[test]
fn history_is_discovered_scanned_once_and_reported_incrementally() {
    let fixture = tempfile::tempdir().unwrap();
    let home = fixture.path().join("home");
    seed_home(&home);
    // The importers read the process environment the way each CLI does; this
    // binary holds one test, so pointing it at the fixture is safe.
    std::env::set_var("HOME", &home);
    for variable in ["CLAUDE_CONFIG_DIR", "CODEX_HOME", "XDG_DATA_HOME"] {
        std::env::remove_var(variable);
    }
    let data_dir = fixture.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    seed_daemon_dir(&data_dir);
    let (daemon, listener) = Daemon::start(DaemonConfig {
        data_dir: data_dir.clone(),
        socket_path: None,
        health_addr: None,
        browser_extension_path: data_dir.join("no-extension"),
        handshake_timeout: Duration::from_millis(400),
    })
    .expect("daemon starts");
    let daemon = std::sync::Arc::new(daemon);
    let serve_daemon = daemon.clone();
    let accept_loop = std::thread::spawn(move || {
        bridged::serve(&serve_daemon, listener).expect("accept loop serves");
    });
    let mut client = Client::connect(&daemon.socket_path, &daemon.state.auth_token);

    let sources = client.call(1, "usage/list_history_sources", None);
    assert!(sources.get("error").is_none(), "{sources}");
    let listed = sources["result"].as_array().unwrap();
    let by_agent = |agent: &str| {
        listed
            .iter()
            .find(|source| source["agent"] == json!(agent))
            .unwrap_or_else(|| panic!("no {agent} source in {sources}"))
    };
    assert_eq!(by_agent("cursor")["capability"], json!("unsupported"));
    assert_eq!(by_agent("cursor")["coverageState"], json!("unsupported"));
    assert!(by_agent("cursor")["coverageReason"].is_string());
    assert_eq!(by_agent("claude")["capability"], json!("supported"));
    assert_eq!(by_agent("claude")["recordsImported"], json!(0));
    assert_eq!(by_agent("codex")["coverageState"], json!("empty"));
    let claude_id = by_agent("claude")["id"].as_str().unwrap().to_owned();

    let first = client.call(2, "usage/scan_history", Some(json!({})));
    assert!(first.get("error").is_none(), "{first}");
    assert_eq!(first["result"]["recordsImported"], json!(1));
    let claude_outcome = first["result"]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|outcome| outcome["agent"] == json!("claude"))
        .unwrap();
    assert_eq!(claude_outcome["recordsImported"], json!(1));
    assert_eq!(claude_outcome["coverage"], json!("complete"));

    let second = client.call(
        3,
        "usage/scan_history",
        Some(json!({"maxRecords": 50, "sourceIds": [claude_id]})),
    );
    assert!(second.get("error").is_none(), "{second}");
    assert_eq!(second["result"]["recordsImported"], json!(0), "a second pass imports nothing new");
    assert_eq!(second["result"]["sources"].as_array().unwrap().len(), 1);

    let after = client.call(4, "usage/list_history_sources", None);
    let claude = after["result"]
        .as_array()
        .unwrap()
        .iter()
        .find(|source| source["agent"] == json!("claude"))
        .unwrap();
    assert_eq!(claude["recordsImported"], json!(1));
    assert!(claude["lastSuccessfulScanAt"].is_string());

    let unknown = client.call(5, "usage/scan_history", Some(json!({"sourceIds": ["nope"]})));
    assert!(unknown.get("error").is_some(), "{unknown}");
    let refused = client.call(6, "usage/scan_history", Some(json!({"bogus": 1})));
    assert_eq!(refused["error"]["code"], json!(-32602));

    // The summary sees the imported observation.
    let summary = client.call(
        7,
        "usage/summary",
        Some(json!({"sinceDay": "2026-03-01", "untilDay": "2026-03-01", "resolution": "day", "includeImported": true})),
    );
    assert_eq!(summary["result"]["importedRecords"], json!(1), "{summary}");
    assert_eq!(summary["result"]["buckets"][0]["totals"]["cacheReadTokens"], json!(40));

    daemon.state.shutting_down.store(true, Ordering::SeqCst);
    accept_loop.join().unwrap();
    daemon.shutdown(Duration::from_secs(5));
}

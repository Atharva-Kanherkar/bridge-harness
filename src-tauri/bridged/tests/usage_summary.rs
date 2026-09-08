//! `usage/summary` over the socket against a seeded data directory: one
//! Codex, one Claude, and one OpenCode session, plus an imported transcript
//! that covers the Claude session.

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
    opencode.advanced = json!({
        "executablePath": data_dir.join("missing-opencode").to_string_lossy(),
    });
    bridge_core::agent_config::save_harness(&db, opencode).unwrap();
    db.execute_batch(
        "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/usage-summary-demo','now');
         INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','c','t','main','/tmp/usage-summary-demo/w','ready','now');
         INSERT INTO sessions(id,workspace_id,harness,label,status,model,provider_session_id) VALUES('codex-1','w','codex','codex','ready','gpt-5','thread-1');
         INSERT INTO sessions(id,workspace_id,harness,label,status,model,provider_session_id) VALUES('claude-1','w','claude','claude','ready','opus','claude-native-1');
         INSERT INTO sessions(id,workspace_id,harness,label,status,model,provider_session_id) VALUES('opencode-1','w','opencode','opencode','ready','anthropic/claude-opus-4-6','ses_1');
         -- Codex: tokens only, priced from the bundled table at read time.
         INSERT INTO usage_ledger(workspace_id,session_id,turn_id,input_tokens,uncached_input_tokens,cache_read_tokens,cache_write_tokens,output_tokens,reasoning_tokens,cost_source,harness,model,source,created_at)
             VALUES('w','codex-1','t1',150,100,50,0,20,5,'model_priced','codex','gpt-5','provider.codex','2026-03-01T12:00:00+00:00');
         -- Claude: the provider's own cost.
         INSERT INTO usage_ledger(workspace_id,session_id,turn_id,input_tokens,uncached_input_tokens,cache_read_tokens,cache_write_tokens,output_tokens,cost_microusd,cost_source,harness,model,source,created_at)
             VALUES('w','claude-1','t2',30,30,300,40,15,12345,'provider_reported','claude','claude-opus-4-6','provider.claude','2026-03-01T13:00:00+00:00');
         -- OpenCode: the step's own cost.
         INSERT INTO usage_ledger(workspace_id,session_id,turn_id,input_tokens,uncached_input_tokens,cache_read_tokens,cache_write_tokens,output_tokens,cost_microusd,cost_source,harness,model,source,created_at)
             VALUES('w','opencode-1','t3',40,5,30,5,8,9000,'provider_reported','opencode','anthropic/claude-opus-4-6','provider.opencode','2026-03-01T14:00:00+00:00');
         -- An imported transcript covering the Claude session.
         INSERT INTO agent_usage_sources(id,agent,provider,location_fingerprint,coverage_state,records_imported,importer_version,created_at,updated_at)
             VALUES('src','claude','anthropic','fp','complete',1,'test','now','now');
         INSERT INTO agent_usage_sessions(id,source_id,native_session_id,agent,provider,created_at,updated_at) VALUES('ases','src','claude-native-1','claude','anthropic','now','now');
         INSERT INTO agent_usage_observations(id,source_id,session_id,native_record_id,occurred_at,model,input_semantics,output_semantics,uncached_input_tokens,cache_read_tokens,cache_write_tokens,output_tokens,exact_total_formula,reported_cost_microusd,importer_version,created_at)
             VALUES('o1','src','ases','msg_1:req_1','2026-03-01T13:00:00+00:00','claude-opus-4-6','exclusive','delta',30,300,40,15,'anthropic_exclusive_input_plus_cache_and_output',12345,'test','now');",
    )
    .unwrap();
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
            json!({
                "protocolVersion": {
                    "major": bridge_protocol::PROTOCOL_VERSION.major,
                    "minor": bridge_protocol::PROTOCOL_VERSION.minor,
                },
                "client": {"name": "usage-test", "version": "0"},
                "authToken": token,
            }),
        );
        assert!(handshake.get("result").is_some(), "handshake failed: {handshake}");
        client
    }

    fn call(&mut self, id: i64, method: &str, params: Value) -> Value {
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
            if frame.get("id").is_some() {
                assert_eq!(frame["id"], json!(id));
                return frame;
            }
        }
    }
}

fn bucket<'a>(summary: &'a Value, harness: &str) -> &'a Value {
    summary["result"]["buckets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|bucket| bucket["harness"] == json!(harness))
        .unwrap_or_else(|| panic!("no {harness} bucket in {summary}"))
}

#[test]
fn usage_summary_reports_totals_cost_and_provenance_per_harness() {
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

    let live = client.call(
        1,
        "usage/summary",
        json!({"sinceDay": "2026-03-01", "untilDay": "2026-03-01", "resolution": "day", "includeImported": false}),
    );
    assert!(live.get("error").is_none(), "{live}");
    let result = &live["result"];
    assert_eq!(result["duplicatesDropped"], json!(0));
    assert_eq!(result["liveRecords"], json!(3));
    assert_eq!(result["importedRecords"], json!(0));
    assert_eq!(result["pricing"]["status"], json!("bundled"));
    assert_eq!(result["buckets"].as_array().unwrap().len(), 3);
    // Sorted by day, hour, harness, model.
    let harnesses: Vec<&str> = result["buckets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|bucket| bucket["harness"].as_str().unwrap())
        .collect();
    assert_eq!(harnesses, ["claude", "codex", "opencode"]);

    let codex = bucket(&live, "codex");
    assert_eq!(codex["model"], json!("gpt-5"));
    assert_eq!(codex["totals"], json!({"uncachedInputTokens": 100, "cacheReadTokens": 50, "cacheWriteTokens": 0, "outputTokens": 20, "reasoningTokens": 5}));
    assert_eq!(codex["costSource"], json!("model_priced"));
    // 100 × $1.25/M + 50 × $0.125/M + 20 × $10/M = 125 + 6.25 + 200 µUSD, half-up.
    assert_eq!(codex["costMicrousd"], json!(331));
    assert_eq!(codex["cacheSavingsMicrousd"], json!(56));
    assert_eq!(codex["records"], json!(1));
    assert_eq!(codex["unpricedRecords"], json!(0));

    let claude = bucket(&live, "claude");
    assert_eq!(claude["costSource"], json!("provider_reported"));
    assert_eq!(claude["costMicrousd"], json!(12_345));
    assert_eq!(claude["totals"]["cacheReadTokens"], json!(300));
    assert_eq!(claude["sessions"], json!(1));

    let opencode = bucket(&live, "opencode");
    assert_eq!(opencode["model"], json!("anthropic/claude-opus-4-6"));
    assert_eq!(opencode["costSource"], json!("provider_reported"));
    assert_eq!(opencode["costMicrousd"], json!(9_000));
    assert_eq!(opencode["totals"]["uncachedInputTokens"], json!(5));

    // With imports, the Claude session's live row yields to its transcript.
    let merged = client.call(
        2,
        "usage/summary",
        json!({"sinceDay": "2026-03-01", "untilDay": "2026-03-01", "resolution": "day", "includeImported": true, "timeZone": "UTC"}),
    );
    assert!(merged.get("error").is_none(), "{merged}");
    assert_eq!(merged["result"]["duplicatesDropped"], json!(1));
    assert_eq!(merged["result"]["liveRecords"], json!(2));
    assert_eq!(merged["result"]["importedRecords"], json!(1));
    let claude = bucket(&merged, "claude");
    assert_eq!(claude["records"], json!(1), "one record, not the live row plus the observation");
    assert_eq!(claude["costMicrousd"], json!(12_345));
    assert_eq!(merged["result"]["sources"][0]["id"], json!("src"));

    // Unknown fields and a malformed window are client errors, not crashes.
    let refused = client.call(
        3,
        "usage/summary",
        json!({"sinceDay": "2026-03-01", "untilDay": "2026-03-01", "resolution": "day", "includeImported": false, "bogus": 1}),
    );
    assert_eq!(refused["error"]["code"], json!(-32602));
    let hourly = client.call(
        4,
        "usage/summary",
        json!({"sinceDay": "2026-03-01", "untilDay": "2026-03-01", "resolution": "hour", "includeImported": false}),
    );
    assert!(hourly["error"]["message"].as_str().unwrap().contains("sinceTime"), "{hourly}");

    // Overrides round-trip and re-price the Codex bucket at read time.
    let set = client.call(
        5,
        "usage/set_price_override",
        json!({"model": "gpt-5", "inputMicrousdPerMtok": 1000000, "outputMicrousdPerMtok": 1000000}),
    );
    assert_eq!(set["result"][0]["model"], json!("gpt-5"));
    let repriced = client.call(
        6,
        "usage/summary",
        json!({"sinceDay": "2026-03-01", "untilDay": "2026-03-01", "resolution": "day", "includeImported": false}),
    );
    // (100 + 50 + 0 + 20) tokens × $1/M.
    assert_eq!(bucket(&repriced, "codex")["costMicrousd"], json!(170));
    assert_eq!(repriced["result"]["pricing"]["overrides"], json!(1));
    let cleared = client.call(7, "usage/clear_price_override", json!({"model": "gpt-5"}));
    assert_eq!(cleared["result"], json!([]));
    let listed = client.call(8, "usage/list_price_overrides", json!(null));
    assert!(listed.get("error").is_some(), "a parameterless method refuses params: {listed}");

    daemon.state.shutting_down.store(true, Ordering::SeqCst);
    accept_loop.join().unwrap();
    daemon.shutdown(Duration::from_secs(5));
}

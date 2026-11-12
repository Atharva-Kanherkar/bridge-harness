//! Workflows end to end, through the daemon: the journeys of
//! `testing/feat-workflows-p1-e2e.md`.
//!
//! Only the model is fake. `BRIDGE_CLAUDE_SIDECAR` runs Bridge's real Claude
//! sidecar (workflow gate, ask relay, control frames) against
//! `testing/fixtures/workflow-e2e/fake-sdk.mjs`, and a fake `claude` CLI first
//! on `PATH` reports two connected connectors. Every write the fake model
//! actually executes is appended to `writes.log` in the run's folder.

use bridged::{Daemon, DaemonConfig, TOKEN_FILE_NAME};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Once};
use std::time::{Duration, Instant};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testing/fixtures")
}

/// Every test in this file runs with the fake model; set once, before any
/// daemon starts, because the environment is process-wide.
fn fake_model() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let e2e = fixtures().join("workflow-e2e");
        let mut paths = vec![e2e.join("bin")];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
        std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
        std::env::set_var("BRIDGE_CLAUDE_SIDECAR", e2e.join("sidecar.mjs"));
    });
}

/// Point OpenCode at a nonexistent executable so background discovery fails
/// immediately instead of spawning a real OpenCode server.
fn seed_data_dir(data_dir: &Path) {
    let db = bridge_core::store::open(&data_dir.join("bridge.db")).unwrap();
    let mut opencode = bridge_core::agent_config::state(&db)
        .unwrap()
        .harnesses
        .into_iter()
        .find(|harness| harness.id == "opencode")
        .unwrap();
    opencode.advanced = json!({ "executablePath": data_dir.join("missing-opencode").to_string_lossy() });
    bridge_core::agent_config::save_harness(&db, opencode).unwrap();
}

struct RunningDaemon {
    daemon: Arc<Daemon>,
    accept_loop: Option<std::thread::JoinHandle<()>>,
}

impl RunningDaemon {
    fn start(data_dir: &Path) -> RunningDaemon {
        let (daemon, listener) = Daemon::start(DaemonConfig {
            data_dir: data_dir.to_path_buf(),
            socket_path: None,
            health_addr: None,
            browser_extension_path: data_dir.join("no-extension"),
            handshake_timeout: Duration::from_millis(400),
        })
        .expect("daemon starts");
        let daemon = Arc::new(daemon);
        let serve_daemon = daemon.clone();
        let accept_loop = std::thread::spawn(move || {
            bridged::serve(&serve_daemon, listener).expect("accept loop serves");
        });
        RunningDaemon { daemon, accept_loop: Some(accept_loop) }
    }

    fn rpc(&self) -> Rpc {
        Rpc::connect(&self.daemon.socket_path, &self.daemon.state.auth_token)
    }

    fn stop(mut self) {
        self.daemon.state.shutting_down.store(true, Ordering::SeqCst);
        if let Some(handle) = self.accept_loop.take() {
            handle.join().unwrap();
        }
        self.daemon.shutdown(Duration::from_secs(5));
    }
}

/// A JSON-RPC client over the daemon's socket.
struct Rpc {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next: i64,
}

impl Rpc {
    fn connect(socket_path: &Path, token: &str) -> Rpc {
        let stream = UnixStream::connect(socket_path).expect("client connects");
        stream.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
        let mut rpc = Rpc { reader: BufReader::new(stream.try_clone().unwrap()), writer: stream, next: 0 };
        let handshake = rpc.request("protocol/handshake", json!({
            "protocolVersion": { "major": bridge_protocol::PROTOCOL_VERSION.major, "minor": bridge_protocol::PROTOCOL_VERSION.minor },
            "client": { "name": "workflows-e2e", "version": "0" },
            "authToken": token,
        }));
        assert!(handshake.is_ok(), "{handshake:?}");
        rpc
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, Value> {
        self.next += 1;
        let mut frame = json!({ "jsonrpc": "2.0", "id": self.next, "method": method });
        // A method without parameters refuses even an empty object.
        if !params.is_null() {
            frame["params"] = params;
        }
        let mut line = serde_json::to_vec(&frame).unwrap();
        line.push(b'\n');
        self.writer.write_all(&line).unwrap();
        loop {
            let mut text = String::new();
            self.reader.read_line(&mut text).expect("a frame arrives");
            assert!(!text.is_empty(), "the daemon closed the connection");
            let frame: Value = serde_json::from_str(&text).unwrap();
            // Notifications interleave; this client polls instead of following them.
            if frame.get("id") != Some(&json!(self.next)) {
                continue;
            }
            return match frame.get("error") {
                Some(error) => Err(error.clone()),
                None => Ok(frame["result"].clone()),
            };
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        self.request(method, params).unwrap_or_else(|error| panic!("{method} failed: {error}"))
    }

    fn runs(&mut self, workflow_id: &str) -> Vec<Value> {
        let listed = self.call("workflows/list_workflow_runs", json!({ "workflowId": workflow_id, "unreadOnly": false, "limit": 50 }));
        listed["runs"].as_array().unwrap().clone()
    }

    /// Poll the run until `until` holds; a slow sidecar start is the usual wait.
    fn wait_run(&mut self, workflow_id: &str, run_id: &str, what: &str, until: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            let runs = self.runs(workflow_id);
            let run = runs.iter().find(|run| run["id"] == json!(run_id)).cloned().unwrap_or(Value::Null);
            if until(&run) {
                return run;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}: {run}");
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// The newest permission card in a session after `after`, by its sequence.
    fn pending_card(&mut self, session_id: &str, after: i64) -> Option<(i64, String)> {
        let events = self.call("sessions/replay_session_events", json!({ "sessionId": session_id, "afterSequence": after, "limit": 1000 }));
        events
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["kind"] == json!("permission.requested"))
            .map(|event| (event["sequence"].as_i64().unwrap(), event.to_string()))
            .last()
    }

    fn wait_card(&mut self, session_id: &str, after: i64, tool: &str) -> i64 {
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            if let Some((sequence, text)) = self.pending_card(session_id, after) {
                assert!(text.contains(tool), "the card is for {tool}: {text}");
                return sequence;
            }
            assert!(Instant::now() < deadline, "timed out waiting for a card for {tool}");
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

fn claude_model(rpc: &mut Rpc) -> String {
    let options = rpc.call("workflows/workflow_options", Value::Null);
    let claude = options["harnesses"].as_array().unwrap().iter().find(|harness| harness["id"] == json!("claude")).unwrap().clone();
    claude["defaultModel"].as_str().map(str::to_owned).unwrap_or_else(|| claude["models"][0]["id"].as_str().unwrap().to_owned())
}

fn budget() -> Value {
    json!({ "maxWallSeconds": 600, "maxTurns": 24, "maxToolCalls": 64, "maxOutputTokens": null, "costCeilingMicrousd": null })
}

fn session_cwd(daemon: &Daemon, session_id: &str) -> PathBuf {
    let db = daemon.core.db.lock().unwrap();
    PathBuf::from(db.query_row("SELECT cwd FROM sessions WHERE id=?1", [session_id], |row| row.get::<_, String>(0)).unwrap())
}

fn writes(folder: &Path) -> Vec<String> {
    std::fs::read_to_string(folder.join("writes.log")).unwrap_or_default().lines().map(str::to_owned).collect()
}

fn effects(daemon: &Daemon, run_id: &str) -> Vec<(String, String)> {
    let db = daemon.core.db.lock().unwrap();
    bridge_core::workflow_run::effects(&db, run_id).unwrap().into_iter().map(|(tool, decision, _)| (tool, decision)).collect()
}

/// The fake clock: one maintenance tick at `now`, dispatching what it claims
/// exactly as the daemon's own loop would.
fn tick_at(daemon: &Daemon, now: DateTime<Utc>) -> Vec<String> {
    let claimed = bridge_core::workflow_trigger::tick(&daemon.core, now).unwrap();
    claimed
        .into_iter()
        .map(|run| {
            let id = run.run.id.clone();
            bridge_core::workflow_trigger::dispatch(daemon.core.clone(), run);
            id
        })
        .collect()
}

const SLACK_POST: &str = "mcp__claude_ai_Slack__slack_send_message";

#[test]
fn e1_a_no_project_workflow_is_drafted_scheduled_approved_and_read() {
    fake_model();
    let fixture = tempfile::tempdir().unwrap();
    seed_data_dir(fixture.path());
    let running = RunningDaemon::start(fixture.path());
    let mut rpc = running.rpc();

    // Draft from a sentence: the recorded reply, validated by Bridge. Nothing is saved.
    let sentence = "Every morning at 8:30, post a summary of yesterday's Linear activity to #team on Slack.";
    let draft = rpc.call("workflows/draft_workflow_from_text", json!({ "text": sentence }));
    let mut definition = draft["draft"]["definition"].clone();
    assert_eq!(definition["trigger"], json!({ "kind": "schedule", "cron": "30 8 * * *", "timezone": "Asia/Kolkata" }), "{draft}");
    assert_eq!(definition["projectId"], Value::Null);
    let grants = definition["grants"].as_array().unwrap();
    assert!(grants.contains(&json!({ "server": "claude.ai Linear", "read": true, "write": "never" })), "{grants:?}");
    assert!(grants.contains(&json!({ "server": "claude.ai Slack", "read": false, "write": "ask" })), "{grants:?}");
    assert_eq!(rpc.call("workflows/list_workflows", Value::Null)["workflows"], json!([]), "a draft saves nothing");

    // The person edits the instructions, then saves: it needs approval.
    definition["steps"][0]["instructions"] = json!(format!("{} E2E:post", definition["steps"][0]["instructions"].as_str().unwrap()));
    let saved = rpc.call("workflows/save_workflow", json!({ "definition": definition }));
    assert_eq!(saved["needsApproval"], true);
    let workflow_id = saved["workflow"]["id"].as_str().unwrap().to_owned();
    let revision_id = saved["workflow"]["revisionId"].as_str().unwrap().to_owned();
    let refused = rpc.request("workflows/run_workflow", json!({ "workflowId": workflow_id, "requestId": "real-before-approval" }));
    assert!(refused.is_err(), "a real run needs approval first");

    // A dry run before approval: every write becomes a preview.
    let dry = rpc.call("workflows/run_workflow", json!({ "workflowId": workflow_id, "requestId": "dry", "dryRun": true }));
    let dry_id = dry["run"]["id"].as_str().unwrap().to_owned();
    let dry = rpc.wait_run(&workflow_id, &dry_id, "the dry run to finish", |run| run["status"] == json!("succeeded") || run["status"] == json!("failed"));
    assert_eq!(dry["status"], json!("succeeded"), "{dry}");
    let home = session_cwd(&running.daemon, dry["sessionId"].as_str().unwrap());
    assert!(writes(&home).is_empty(), "a dry run executes no write");
    assert_eq!(effects(&running.daemon, &dry_id), vec![(SLACK_POST.to_owned(), "previewed".to_owned())]);

    // Approved, the schedule fires at the next slot on the fake clock.
    rpc.call("workflows/approve_workflow", json!({ "workflowId": workflow_id, "revisionId": revision_id }));
    let summary = rpc.call("workflows/list_workflows", Value::Null)["workflows"][0].clone();
    let next: DateTime<Utc> = summary["nextRunAt"].as_str().expect("an approved schedule has a next run").parse().unwrap();
    assert!(tick_at(&running.daemon, next - ChronoDuration::seconds(1)).is_empty(), "nothing fires before the slot");
    let fired = tick_at(&running.daemon, next + ChronoDuration::seconds(1));
    assert_eq!(fired.len(), 1, "one slot, one run");
    let run_id = fired[0].clone();

    // The Slack write waits for the person; they allow it once.
    let run = rpc.wait_run(&workflow_id, &run_id, "the write to wait for approval", |run| run["status"] == json!("waiting_approval"));
    assert_eq!(run["triggerKind"], json!("schedule"));
    let session_id = run["sessionId"].as_str().unwrap().to_owned();
    let card = rpc.wait_card(&session_id, 0, "slack_send_message");
    rpc.call("approvals/resolve_approval", json!({ "sessionId": session_id, "eventId": card, "decision": "accept" }));

    let run = rpc.wait_run(&workflow_id, &run_id, "the run to finish", |run| run["status"] == json!("succeeded") || run["status"] == json!("failed"));
    assert_eq!(run["status"], json!("succeeded"), "{run}");
    assert_eq!(run["summary"], json!("Posted yesterday's Linear summary to #team."));
    assert_eq!(writes(&home), vec!["slack_send_message".to_owned()], "exactly one write executed");
    assert_eq!(effects(&running.daemon, &run_id), vec![(SLACK_POST.to_owned(), "approved".to_owned())]);

    // In the inbox, unread; openable and forkable into a visible chat.
    let unread = rpc.call("workflows/list_workflow_runs", json!({ "unreadOnly": true, "limit": 50 }));
    assert!(unread["runs"].as_array().unwrap().iter().any(|candidate| candidate["id"] == json!(run_id)));
    let forest = rpc.call("sessions/get_session_forest", json!({ "sessionId": session_id }));
    let last = forest["entries"].as_array().unwrap().iter().max_by_key(|entry| entry["sequence"].as_i64().unwrap()).unwrap()["id"].clone();
    let fork = rpc.call("sessions/fork_session", json!({ "sessionId": session_id, "entryId": last, "title": "Morning post (continued)", "worktreePolicy": "shared" }));
    let fork_id = fork["sessionId"].as_str().unwrap();
    let forked = fork["state"]["sessions"].as_array().unwrap().iter().find(|session| session["id"] == json!(fork_id)).unwrap().clone();
    assert_ne!(forked["kind"], json!("workflow"), "the continuation is an ordinary chat: {forked}");

    rpc.call("workflows/mark_workflow_runs_read", json!({ "runIds": [run_id] }));
    let unread = rpc.call("workflows/list_workflow_runs", json!({ "unreadOnly": true, "limit": 50 }));
    assert!(!unread["runs"].as_array().unwrap().iter().any(|candidate| candidate["id"] == json!(run_id)));

    running.stop();
}

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git").arg("-C").arg(repo).args(args).output().unwrap();
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn e2_a_project_workflow_edits_its_own_worktree_and_leaves_the_checkout_alone() {
    fake_model();
    let fixture = tempfile::tempdir().unwrap();
    let data_dir = fixture.path().join("data");
    let repo = fixture.path().join("repo");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "e2e@bridge.test"]);
    git(&repo, &["config", "user.name", "Bridge E2E"]);
    std::fs::write(repo.join("README.md"), "# Demo\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let status_before = git(&repo, &["status", "--porcelain"]);

    seed_data_dir(&data_dir);
    let running = RunningDaemon::start(&data_dir);
    let mut rpc = running.rpc();
    let state = rpc.call("projects/add_project", json!({ "path": repo.to_string_lossy() }));
    let project_id = state["projects"].as_array().unwrap()[0]["id"].as_str().unwrap().to_owned();
    let model = claude_model(&mut rpc);

    let saved = rpc.call("workflows/save_workflow", json!({ "definition": {
        "name": "Changelog",
        "projectId": project_id,
        "steps": [{ "harness": "claude", "model": model, "instructions": "Add a changelog. E2E:edit" }],
        "trigger": { "kind": "manual" },
        "files": "worktree",
        "budget": budget(),
    }}));
    let workflow_id = saved["workflow"]["id"].as_str().unwrap().to_owned();
    rpc.call("workflows/approve_workflow", json!({ "workflowId": workflow_id, "revisionId": saved["workflow"]["revisionId"] }));
    let started = rpc.call("workflows/run_workflow", json!({ "workflowId": workflow_id, "requestId": "edit" }));
    let run_id = started["run"]["id"].as_str().unwrap().to_owned();
    let run = rpc.wait_run(&workflow_id, &run_id, "the edit to finish", |run| {
        ["succeeded", "failed", "unverified"].iter().any(|status| run["status"] == json!(status))
    });
    assert_eq!(run["status"], json!("succeeded"), "{run}");

    let cwd = session_cwd(&running.daemon, run["sessionId"].as_str().unwrap());
    assert_ne!(cwd, repo, "the run works in its own worktree");
    assert_eq!(std::fs::read_to_string(cwd.join("CHANGELOG.md")).unwrap(), "- Workflows can edit code.\n");
    assert!(!repo.join("CHANGELOG.md").exists(), "the checkout does not get the edit");
    assert_eq!(git(&repo, &["status", "--porcelain"]), status_before, "the checkout's status is unchanged");

    running.stop();
}

#[test]
fn e3_a_daemon_killed_after_one_write_reclaims_the_run_without_repeating_it() {
    fake_model();
    let fixture = tempfile::tempdir().unwrap();
    let data_dir = fixture.path();
    seed_data_dir(data_dir);

    // A real daemon process, so it can be killed for real.
    let mut child = Command::new(env!("CARGO_BIN_EXE_bridged"))
        .args(["--data-dir", data_dir.to_str().unwrap(), "--health-addr", "none"])
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let socket_path = data_dir.join(bridged::SOCKET_FILE_NAME);
    let token_path = data_dir.join(TOKEN_FILE_NAME);
    let deadline = Instant::now() + Duration::from_secs(60);
    while !socket_path.exists() || !token_path.exists() {
        assert!(Instant::now() < deadline, "the daemon process never came up");
        std::thread::sleep(Duration::from_millis(50));
    }
    let token = std::fs::read_to_string(&token_path).unwrap().trim().to_owned();
    let mut rpc = Rpc::connect(&socket_path, &token);
    let model = claude_model(&mut rpc);

    let saved = rpc.call("workflows/save_workflow", json!({ "definition": {
        "name": "Post twice",
        "steps": [{ "harness": "claude", "model": model, "instructions": "Post, then react. E2E:post E2E:hold" }],
        "trigger": { "kind": "manual" },
        "files": "none",
        "grants": [{ "server": "claude.ai Slack", "read": true, "write": "ask" }, { "server": "claude.ai Linear", "read": true, "write": "never" }],
        "budget": budget(),
    }}));
    let workflow_id = saved["workflow"]["id"].as_str().unwrap().to_owned();
    rpc.call("workflows/approve_workflow", json!({ "workflowId": workflow_id, "revisionId": saved["workflow"]["revisionId"] }));
    let started = rpc.call("workflows/run_workflow", json!({ "workflowId": workflow_id, "requestId": "twice" }));
    let run_id = started["run"]["id"].as_str().unwrap().to_owned();

    // One write approved and executed; the second waits when the daemon dies.
    let run = rpc.wait_run(&workflow_id, &run_id, "the first write to wait", |run| run["status"] == json!("waiting_approval"));
    let session_id = run["sessionId"].as_str().unwrap().to_owned();
    let first = rpc.wait_card(&session_id, 0, "slack_send_message");
    rpc.call("approvals/resolve_approval", json!({ "sessionId": session_id, "eventId": first, "decision": "accept" }));
    rpc.wait_card(&session_id, first, "slack_add_reaction");
    let home = data_dir.join("workflows").join(&workflow_id);
    assert_eq!(writes(&home), vec!["slack_send_message".to_owned()]);

    child.kill().unwrap();
    child.wait().unwrap();
    drop(rpc);

    // A new owner of the directory. Nothing happens until the lease expires;
    // then the run is reclaimed, and the executed write is not repeated.
    let running = RunningDaemon::start(data_dir);
    let mut rpc = running.rpc();
    assert!(rpc.runs(&workflow_id).iter().any(|run| run["id"] == json!(run_id) && run["status"] == json!("waiting_approval")));
    tick_at(&running.daemon, Utc::now() + ChronoDuration::minutes(16));
    let run = rpc.wait_run(&workflow_id, &run_id, "the reclaim", |run| run["status"] == json!("failed"));
    assert_eq!(run["failureCode"], json!("lease_expired"));
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(writes(&home), vec!["slack_send_message".to_owned()], "the executed write is not repeated");
    assert_eq!(effects(&running.daemon, &run_id), vec![(SLACK_POST.to_owned(), "approved".to_owned())]);

    running.stop();
}

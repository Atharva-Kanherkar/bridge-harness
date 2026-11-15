//! Usage-limit recovery end to end, through the daemon: the journeys of
//! `testing/feat-limit-auto-resume.md`.
//!
//! Only the model is fake. `BRIDGE_CLAUDE_SIDECAR` runs Bridge's real Claude
//! sidecar against `testing/fixtures/limit-recovery-e2e/fake-sdk.mjs`, which
//! fails a turn with Claude's recorded limit banner when the message says
//! `E2E:limit`, and logs every turn's model and prompt. The daemon's own
//! recovery loop does the acting; the test only moves a row's due time into
//! the past to stand in for hours passing.

use bridged::{Daemon, DaemonConfig};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Once};
use std::time::{Duration, Instant};

const RESUME_PROMPT: &str = bridge_core::limit_recovery::CONTINUE_PROMPT;
const BACKUP_PROMPT: &str = bridge_core::limit_recovery::CONTINUE_ON_BACKUP_PROMPT;
const BANNER: &str = "You've hit your session limit · resets 7:50pm (Asia/Tokyo)";

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testing/fixtures")
}

fn fake_model() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // The workflow fixture's fake `claude` CLI answers discovery.
        let mut paths = vec![fixtures().join("workflow-e2e/bin")];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
        std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
        std::env::set_var("BRIDGE_CLAUDE_SIDECAR", fixtures().join("limit-recovery-e2e/sidecar.mjs"));
        std::env::set_var("BRIDGE_LIMIT_RECOVERY_TICK_MS", "150");
        std::env::set_var("BRIDGE_LIMIT_RECOVERY_NO_CAFFEINATE", "1");
    });
}

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
            "client": { "name": "limit-recovery-e2e", "version": "0" },
            "authToken": token,
        }));
        assert!(handshake.is_ok(), "{handshake:?}");
        rpc
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, Value> {
        self.next += 1;
        let mut frame = json!({ "jsonrpc": "2.0", "id": self.next, "method": method });
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

    fn recovery(&mut self, session_id: &str) -> Value {
        self.call("sessions/limit_recovery_status", json!({ "sessionId": session_id }))["recovery"].clone()
    }

    fn wait_recovery(&mut self, session_id: &str, what: &str, until: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            let recovery = self.recovery(session_id);
            if until(&recovery) {
                return recovery;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}: {recovery}");
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// Two distinct Claude models from the catalog the daemon advertises.
fn claude_models(rpc: &mut Rpc) -> (String, String) {
    let options = rpc.call("workflows/workflow_options", Value::Null);
    let claude = options["harnesses"].as_array().unwrap().iter().find(|harness| harness["id"] == json!("claude")).unwrap().clone();
    let models: Vec<String> = claude["models"].as_array().unwrap().iter().map(|model| model["id"].as_str().unwrap().to_owned()).collect();
    assert!(models.len() >= 2, "the catalog lists two Claude models: {models:?}");
    (models[0].clone(), models[1].clone())
}

fn supported_efforts(daemon: &Daemon, model: &str) -> Vec<String> {
    daemon.core.adapter_registry.descriptors().into_iter()
        .find(|adapter| adapter.id == "claude").unwrap()
        .models.into_iter().find(|option| option.id == model).unwrap()
        .supported_effort_levels
}

fn create_chat(rpc: &mut Rpc, model: &str) -> String {
    let before: Vec<String> = sessions(rpc);
    rpc.call("sessions/create_chat", json!({ "harness": "claude", "model": model }));
    sessions(rpc).into_iter().find(|id| !before.contains(id)).expect("a new chat")
}

fn sessions(rpc: &mut Rpc) -> Vec<String> {
    let state = rpc.call("state/get_state", Value::Null);
    state["sessions"].as_array().map(|sessions| sessions.iter().map(|session| session["id"].as_str().unwrap().to_owned()).collect()).unwrap_or_default()
}

fn send(rpc: &mut Rpc, session_id: &str, text: &str) {
    rpc.call("sessions/submit_input", json!({ "sessionId": session_id, "text": text }));
}

fn turn_log(daemon: &Daemon, session_id: &str) -> Vec<(String, String)> {
    let cwd: String = daemon.core.db.lock().unwrap().query_row("SELECT cwd FROM sessions WHERE id=?1", [session_id], |row| row.get(0)).unwrap();
    std::fs::read_to_string(Path::new(&cwd).join("limit-e2e.log"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.split_once('\t').map(|(model, prompt)| (model.to_owned(), prompt.to_owned())))
        .collect()
}

fn wait_log(daemon: &Daemon, session_id: &str, what: &str, until: impl Fn(&[(String, String)]) -> bool) -> Vec<(String, String)> {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let log = turn_log(daemon, session_id);
        if until(&log) {
            return log;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}: {log:?}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn session_model(daemon: &Daemon, session_id: &str) -> (String, Option<String>, Option<String>) {
    daemon.core.db.lock().unwrap().query_row("SELECT harness,model,effort FROM sessions WHERE id=?1", [session_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap()
}

/// Hours pass: the pending row is due now. The daemon's loop does the rest.
fn fast_forward(daemon: &Daemon, session_id: &str) {
    daemon.core.db.lock().unwrap().execute(
        "UPDATE limit_recoveries SET due_at=?2 WHERE session_id=?1 AND state='pending'",
        rusqlite::params![session_id, (Utc::now() - chrono::Duration::seconds(1)).to_rfc3339()],
    ).unwrap();
}

#[test]
fn j1_resume_waits_for_the_stated_reset_then_continues_on_the_same_model() {
    fake_model();
    let fixture = tempfile::tempdir().unwrap();
    seed_data_dir(fixture.path());
    let running = RunningDaemon::start(fixture.path());
    let mut rpc = running.rpc();
    let (model, _) = claude_models(&mut rpc);

    let saved = rpc.call("config/save_limit_recovery_settings", json!({ "settings": { "mode": "resume" } }));
    assert_eq!(saved["mode"], json!("resume"));
    let session = create_chat(&mut rpc, &model);
    let observed_at = Utc::now();
    send(&mut rpc, &session, "Refactor the parser. E2E:limit");

    // The limit is read from Claude's own banner, reset in Asia/Tokyo.
    let pending = rpc.wait_recovery(&session, "a pending recovery", |recovery| recovery["state"] == json!("pending"));
    let expected = bridge_core::provider_limit::detect_at(BANNER, observed_at).unwrap().reset_at.unwrap();
    let reset: DateTime<Utc> = pending["resetAt"].as_str().unwrap().parse().unwrap();
    assert!((reset - expected).num_seconds().abs() <= 60, "{reset} vs {expected}");
    let due: DateTime<Utc> = pending["dueAt"].as_str().unwrap().parse().unwrap();
    assert_eq!(due - reset, bridge_core::limit_recovery::RESET_GRACE);
    assert_eq!(pending["switchesModel"], json!(false));
    assert_eq!(pending["target"]["model"], json!(model));

    // Before the reset, nothing is sent.
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(turn_log(&running.daemon, &session).len(), 1, "only the person's turn ran");

    fast_forward(&running.daemon, &session);
    let log = wait_log(&running.daemon, &session, "the continue turn", |log| log.len() >= 2);
    assert_eq!(log[1].0, model, "{log:?}");
    assert!(log[1].1.ends_with(RESUME_PROMPT), "{log:?}");
    let done = rpc.wait_recovery(&session, "the row to finish", |recovery| recovery["state"] == json!("done"));
    assert_eq!(done["outcome"], json!("continued after the reset"));
    assert_eq!(session_model(&running.daemon, &session).1.as_deref(), Some(model.as_str()));

    // Exactly once.
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(turn_log(&running.daemon, &session).len(), 2);
    running.stop();
}

#[test]
fn j2_backup_switches_model_and_effort_now_and_continues() {
    fake_model();
    let fixture = tempfile::tempdir().unwrap();
    seed_data_dir(fixture.path());
    let running = RunningDaemon::start(fixture.path());
    let mut rpc = running.rpc();
    let (primary, backup) = claude_models(&mut rpc);

    let refused = rpc.request("config/save_limit_recovery_settings", json!({ "settings": { "mode": "backup" } }));
    assert!(refused.is_err(), "backup mode needs a backup");
    let refused = rpc.request("config/save_limit_recovery_settings", json!({ "settings": { "mode": "backup", "backup": { "harness": "claude", "model": "not-a-model" } } }));
    assert!(refused.is_err(), "a backup model must be one the harness lists");
    // Prefer a backup that takes an effort, so the switch carries one.
    let models: Vec<String> = running.daemon.core.adapter_registry.descriptors().into_iter()
        .find(|adapter| adapter.id == "claude").unwrap().models.into_iter().map(|option| option.id).collect();
    let backup = models.iter().find(|model| **model != primary && !supported_efforts(&running.daemon, model).is_empty()).cloned().unwrap_or(backup);
    let efforts = supported_efforts(&running.daemon, &backup);
    let effort = efforts.last().cloned();
    let unsupported = ["low", "medium", "high", "xhigh"].into_iter().find(|level| !efforts.iter().any(|supported| supported == level));
    if let Some(level) = unsupported {
        let refused = rpc.request("config/save_limit_recovery_settings", json!({ "settings": { "mode": "backup", "backup": { "harness": "claude", "model": backup, "effort": level } } }));
        assert!(refused.is_err(), "an effort the backup model does not support is refused at save");
    }
    rpc.call("config/save_limit_recovery_settings", json!({ "settings": { "mode": "backup", "backup": { "harness": "claude", "model": backup, "effort": effort } } }));

    let session = create_chat(&mut rpc, &primary);
    send(&mut rpc, &session, "Write the migration. E2E:limit");

    let done = rpc.wait_recovery(&session, "the row to finish", |recovery| {
        !recovery.is_null() && recovery["state"] != json!("pending") && recovery["state"] != json!("acting")
    });
    assert_eq!(done["state"], json!("done"), "{done}");
    let log = wait_log(&running.daemon, &session, "the turn on the backup", |log| log.len() >= 2);
    assert_eq!(log[0].0, primary);
    assert_eq!(log[1].0, backup, "{log:?}");
    assert!(log[1].1.ends_with(BACKUP_PROMPT), "{log:?}");
    assert_eq!(done["switchesModel"], json!(true));
    let (harness, model, effort_now) = session_model(&running.daemon, &session);
    assert_eq!((harness.as_str(), model.as_deref()), ("claude", Some(backup.as_str())));
    assert_eq!(effort_now, effort);

    // The backup runs out too: it waits for its own reset, on the backup.
    send(&mut rpc, &session, "Keep going. E2E:limit");
    let pending = rpc.wait_recovery(&session, "a resume on the backup", |recovery| recovery["state"] == json!("pending"));
    assert_eq!(pending["switchesModel"], json!(false));
    assert_eq!(pending["target"]["model"], json!(backup));
    assert_eq!(pending["attempts"], json!(1), "a person's own message starts a new count");
    running.stop();
}

#[test]
fn j3_a_person_writing_first_cancels_and_resume_now_acts_at_once() {
    fake_model();
    let fixture = tempfile::tempdir().unwrap();
    seed_data_dir(fixture.path());
    let running = RunningDaemon::start(fixture.path());
    let mut rpc = running.rpc();
    let (model, _) = claude_models(&mut rpc);
    rpc.call("config/save_limit_recovery_settings", json!({ "settings": { "mode": "resume" } }));

    // Cancelled by the person's own message.
    let session = create_chat(&mut rpc, &model);
    send(&mut rpc, &session, "First try. E2E:limit");
    rpc.wait_recovery(&session, "a pending recovery", |recovery| recovery["state"] == json!("pending"));
    send(&mut rpc, &session, "Never mind, do this instead.");
    let cancelled = rpc.wait_recovery(&session, "the cancel", |recovery| recovery["state"] == json!("cancelled"));
    assert_eq!(cancelled["outcome"], json!("you sent a message"));
    fast_forward(&running.daemon, &session);
    std::thread::sleep(Duration::from_millis(800));
    let log = turn_log(&running.daemon, &session);
    assert!(log.iter().all(|(_, prompt)| !prompt.ends_with(RESUME_PROMPT)), "{log:?}");

    // Cancelled from the banner.
    let other = create_chat(&mut rpc, &model);
    send(&mut rpc, &other, "Second. E2E:limit");
    rpc.wait_recovery(&other, "a pending recovery", |recovery| recovery["state"] == json!("pending"));
    let cancelled = rpc.call("sessions/cancel_limit_recovery", json!({ "sessionId": other }));
    assert_eq!(cancelled["recovery"]["state"], json!("cancelled"));

    // "Resume now" from the banner, without waiting for the reset.
    let third = create_chat(&mut rpc, &model);
    send(&mut rpc, &third, "Third. E2E:limit");
    rpc.wait_recovery(&third, "a pending recovery", |recovery| recovery["state"] == json!("pending"));
    let resumed = rpc.call("sessions/resume_limit_recovery", json!({ "sessionId": third }));
    assert_eq!(resumed["recovery"]["state"], json!("done"));
    let log = wait_log(&running.daemon, &third, "the continue turn", |log| log.len() >= 2);
    assert!(log[1].1.ends_with(RESUME_PROMPT), "{log:?}");
    assert!(rpc.request("sessions/resume_limit_recovery", json!({ "sessionId": third })).is_err(), "nothing left to resume");
    running.stop();
}

#[test]
fn j4_off_by_default_changes_nothing() {
    fake_model();
    let fixture = tempfile::tempdir().unwrap();
    seed_data_dir(fixture.path());
    let running = RunningDaemon::start(fixture.path());
    let mut rpc = running.rpc();
    let (model, _) = claude_models(&mut rpc);
    assert_eq!(rpc.call("config/get_limit_recovery_settings", Value::Null)["mode"], json!("off"));
    let session = create_chat(&mut rpc, &model);
    send(&mut rpc, &session, "E2E:limit");
    wait_log(&running.daemon, &session, "the limited turn", |log| !log.is_empty());
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(rpc.recovery(&session), Value::Null);
    assert_eq!(turn_log(&running.daemon, &session).len(), 1);
    running.stop();
}

#[test]
fn j5_a_pending_recovery_survives_a_restart_and_fires_after_it() {
    fake_model();
    let fixture = tempfile::tempdir().unwrap();
    seed_data_dir(fixture.path());
    let running = RunningDaemon::start(fixture.path());
    let mut rpc = running.rpc();
    let (model, _) = claude_models(&mut rpc);
    rpc.call("config/save_limit_recovery_settings", json!({ "settings": { "mode": "resume" } }));
    let session = create_chat(&mut rpc, &model);
    send(&mut rpc, &session, "Long task. E2E:limit");
    rpc.wait_recovery(&session, "a pending recovery", |recovery| recovery["state"] == json!("pending"));
    drop(rpc);
    running.stop();

    // Bridge quit and came back before the reset.
    let running = RunningDaemon::start(fixture.path());
    let mut rpc = running.rpc();
    assert_eq!(rpc.recovery(&session)["state"], json!("pending"));
    fast_forward(&running.daemon, &session);
    let log = wait_log(&running.daemon, &session, "the continue turn after restart", |log| log.len() >= 2);
    assert!(log[1].1.ends_with(RESUME_PROMPT), "{log:?}");
    rpc.wait_recovery(&session, "the row to finish", |recovery| recovery["state"] == json!("done"));
    running.stop();
}

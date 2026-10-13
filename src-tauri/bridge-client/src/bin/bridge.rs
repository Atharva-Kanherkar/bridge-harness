//! The `bridge` CLI.
//!
//! ```text
//! bridge exec --json --harness <id> [flags] "<prompt>"   run one turn, stream JSONL events
//! bridge exec --json [flags] --method <m> [--params <json>]
//!                                                        call one protocol method
//! bridge learning run --database <bridge.db> --trigger <...>
//! bridge meter --json                                    print the menu-bar meter registry
//! ```
//!
//! `meter` is the `codexbar serve` equivalent for scripts: the static provider
//! registry (live vs planned) plus the adaptive cadence constants. Live quota
//! windows ride the account-usage event channel, not this command.
//!
//! `exec` is the CI one-shot: it attaches to a running daemon when one owns
//! the data directory, and otherwise hosts the runtime itself for exactly the
//! duration of the command — CI never keeps a user daemon alive. Every event
//! and result is a JSON line on stdout; diagnostics go to stderr. The
//! `--timeout` budget covers the whole command, setup calls included.

use bridge_client::{ClientError, DaemonClient, Endpoint, SessionEventStream};
use bridge_protocol::MethodName;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

/// After `turn.completed`, how long to keep draining for a trailing failure
/// event — some providers report the turn's failure only after completing it.
const FAILURE_GRACE: Duration = Duration::from_secs(2);

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("exec") => exec(&args[1..]),
        Some("learning") if args.get(1).map(String::as_str) == Some("run") => {
            learning_run(&args[2..])
        }
        Some("meter") => meter(&args[1..]),
        Some("status") => status(&args[1..]),
        Some("sessions") => sessions(&args[1..]),
        Some("approval") => approval(&args[1..]),
        _ => {
            eprintln!(
                "usage:\n  bridge exec --json [--data-dir <dir>] [--timeout <secs>] \
                 (--harness <id> [--model <id>] \"<prompt>\" | --method <name> [--params <json>])\n  \
                 bridge learning run --database <bridge.db> --trigger \
                 <manual|in-app|codex:ID|claude:ID|opencode:ID> [--credential-ref <reference>]\n  \
                 bridge meter --json\n  \
                 bridge status --json [--watch] [--data-dir <dir>]\n  \
                 bridge sessions --json [--data-dir <dir>]\n  \
                 bridge approval <allow-once|allow-session|decline> <sessionId> <eventId> [--data-dir <dir>]"
            );
            ExitCode::from(2)
        }
    }
}

// --- meter --------------------------------------------------------------------

/// Print the static meter registry as JSON: live vs planned providers plus the
/// adaptive cadence constants CodexBar's `codexbar serve` exposes for bars.
fn meter(args: &[String]) -> ExitCode {
    if args != ["--json"] {
        eprintln!("usage:\n  bridge meter --json");
        return ExitCode::from(2);
    }
    match serde_json::to_string_pretty(&bridge_core::meter::registry_snapshot()) {
        Ok(document) => {
            println!("{document}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("bridge meter: {error}");
            ExitCode::from(1)
        }
    }
}

// --- status, sessions, approval -------------------------------------------------
//
// Read-mostly commands for desktop surfaces such as the Omarchy bar. They
// attach to a running `bridged` and never host one: a bar polling for counts
// must not boot the whole runtime when Bridge is closed.

/// The data dir the desktop app uses by default, so these commands need no
/// flags when run from a bar or a launcher.
fn default_data_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("BRIDGE_DATA_DIR") {
        return Some(PathBuf::from(dir));
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    if cfg!(target_os = "macos") {
        return Some(home.join("Library/Application Support/dev.bridge.deck"));
    }
    let base = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"));
    Some(base.join("dev.bridge.deck"))
}

#[derive(Debug, PartialEq, Eq)]
struct SurfaceFlags {
    data_dir: PathBuf,
    watch: bool,
    positional: Vec<String>,
}

fn parse_surface_flags(args: &[String], allow_watch: bool) -> Result<SurfaceFlags, String> {
    let mut data_dir = None;
    let mut watch = false;
    let mut json = false;
    let mut positional = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--watch" if allow_watch => watch = true,
            "--data-dir" => data_dir = Some(PathBuf::from(iter.next().ok_or("--data-dir requires a value")?)),
            other if other.starts_with("--") => return Err(format!("unknown flag {other}")),
            other => positional.push(other.to_owned()),
        }
    }
    if !json && positional.is_empty() {
        return Err("pass --json".into());
    }
    let data_dir = data_dir.or_else(default_data_dir).ok_or("--data-dir is required (or set BRIDGE_DATA_DIR)")?;
    Ok(SurfaceFlags { data_dir, watch, positional })
}

fn attach(data_dir: &Path) -> Option<DaemonClient> {
    let endpoint = Endpoint::for_data_dir(data_dir).ok()?;
    DaemonClient::connect_with_timeout(&endpoint, bridge_client::DEFAULT_CONNECT_TIMEOUT).ok()
}

/// Statuses that mean an agent is busy, and the one that means it waits on you.
const WORKING: &[&str] = &["working", "starting", "resuming", "checkpointing"];

/// The fleet summary a bar shows. Only live sessions count.
fn summarize(state: &Value) -> Value {
    let live: Vec<&Value> = state["sessions"]
        .as_array()
        .map(|sessions| sessions.iter().filter(|session| session["endedAt"].is_null()).collect())
        .unwrap_or_default();
    let status = |session: &Value| session["status"].as_str().unwrap_or_default().to_owned();
    let working = live.iter().filter(|session| WORKING.contains(&status(session).as_str())).count();
    let waiting = live.iter().filter(|session| status(session) == "waiting").count();
    let sessions: Vec<Value> = live
        .iter()
        .filter(|session| WORKING.contains(&status(session).as_str()) || status(session) == "waiting")
        .map(|session| json!({
            "id": session["id"],
            "label": session["title"].as_str().filter(|title| !title.is_empty()).unwrap_or(session["label"].as_str().unwrap_or_default()),
            "harness": session["harness"],
            "status": session["status"],
            "parentSessionId": session["parentSessionId"],
        }))
        .collect();
    json!({ "running": true, "working": working, "waiting": waiting, "sessions": sessions })
}

fn stopped_summary() -> Value {
    json!({ "running": false, "working": 0, "waiting": 0, "sessions": [] })
}

fn fetch_summary(client: &DaemonClient) -> Option<Value> {
    client.call(MethodName::GetState, None).ok().map(|state| summarize(&state))
}

fn status(args: &[String]) -> ExitCode {
    let flags = match parse_surface_flags(args, true) {
        Ok(flags) => flags,
        Err(message) => {
            eprintln!("bridge status: {message}");
            return ExitCode::from(2);
        }
    };
    if !flags.watch {
        let summary = attach(&flags.data_dir).and_then(|client| fetch_summary(&client)).unwrap_or_else(stopped_summary);
        emit(&summary);
        return ExitCode::SUCCESS;
    }
    // Watch: one line per change. Re-query on every daemon notification,
    // and every few seconds regardless, so a missed event only delays.
    let mut last: Option<Value> = None;
    let mut publish = |summary: Value| {
        if last.as_ref() != Some(&summary) {
            emit(&summary);
            last = Some(summary);
        }
    };
    loop {
        let Some(client) = attach(&flags.data_dir) else {
            publish(stopped_summary());
            std::thread::sleep(Duration::from_secs(5));
            continue;
        };
        let notifications = client.subscribe();
        loop {
            match fetch_summary(&client) {
                Some(summary) => publish(summary),
                None => break,
            }
            let _ = notifications.recv_timeout(Duration::from_secs(5));
            // Coalesce a burst (a streaming turn) into one re-query.
            while notifications.recv_timeout(Duration::from_millis(150)).is_ok() {}
        }
        publish(stopped_summary());
    }
}

fn sessions(args: &[String]) -> ExitCode {
    let flags = match parse_surface_flags(args, false) {
        Ok(flags) => flags,
        Err(message) => {
            eprintln!("bridge sessions: {message}");
            return ExitCode::from(2);
        }
    };
    let Some(client) = attach(&flags.data_dir) else {
        eprintln!("bridge sessions: Bridge is not running");
        return ExitCode::FAILURE;
    };
    match client.call(MethodName::GetState, None) {
        Ok(state) => {
            emit(&state["sessions"]);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("bridge sessions: {error}");
            ExitCode::FAILURE
        }
    }
}

/// `allow-once` / `allow-session` / `decline` to the wire's decision names.
fn approval_decision(action: &str) -> Option<&'static str> {
    match action {
        "allow-once" => Some("accept"),
        "allow-session" => Some("acceptForSession"),
        "decline" => Some("decline"),
        _ => None,
    }
}

/// Resolve one approval through the same method the in-app button calls, so
/// the policy engine stays the only authority and a second resolution of the
/// same approval comes back as `alreadyResolved` rather than acting twice.
fn approval(args: &[String]) -> ExitCode {
    let flags = match parse_surface_flags(args, false) {
        Ok(flags) => flags,
        Err(message) => {
            eprintln!("bridge approval: {message}");
            return ExitCode::from(2);
        }
    };
    let [action, session_id, event_id] = flags.positional.as_slice() else {
        eprintln!("usage: bridge approval <allow-once|allow-session|decline> <sessionId> <eventId>");
        return ExitCode::from(2);
    };
    let (Some(decision), Ok(event_id)) = (approval_decision(action), event_id.parse::<i64>()) else {
        eprintln!("usage: bridge approval <allow-once|allow-session|decline> <sessionId> <eventId>");
        return ExitCode::from(2);
    };
    let Some(client) = attach(&flags.data_dir) else {
        eprintln!("bridge approval: Bridge is not running");
        return ExitCode::FAILURE;
    };
    let params = json!({ "sessionId": session_id, "eventId": event_id, "decision": decision });
    match client.call(MethodName::ResolveApproval, Some(params)) {
        Ok(result) => {
            emit(&result);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("bridge approval: {error}");
            ExitCode::FAILURE
        }
    }
}

// --- exec ---------------------------------------------------------------------

#[derive(Debug)]
struct ExecFlags {
    data_dir: PathBuf,
    harness: Option<String>,
    model: Option<String>,
    timeout: Duration,
    method: Option<String>,
    params: Option<Value>,
    prompt: Option<String>,
}

fn parse_exec_flags(args: &[String]) -> Result<ExecFlags, String> {
    let mut data_dir: Option<PathBuf> = None;
    let mut harness = None;
    let mut model = None;
    let mut timeout = Duration::from_secs(600);
    let mut json = false;
    let mut method = None;
    let mut params = None;
    let mut prompt = None;
    let mut iter = args.iter();
    while let Some(flag) = iter.next() {
        let mut value = |flag: &str| {
            iter.next().cloned().ok_or_else(|| format!("{flag} requires a value"))
        };
        match flag.as_str() {
            "--json" => json = true,
            "--data-dir" => data_dir = Some(PathBuf::from(value("--data-dir")?)),
            "--harness" => harness = Some(value("--harness")?),
            "--model" => model = Some(value("--model")?),
            "--timeout" => {
                timeout = Duration::from_secs(
                    value("--timeout")?
                        .parse()
                        .map_err(|_| "--timeout must be a number of seconds".to_owned())?,
                )
            }
            "--method" => method = Some(value("--method")?),
            "--params" => {
                params = Some(
                    serde_json::from_str(&value("--params")?)
                        .map_err(|error| format!("--params must be JSON: {error}"))?,
                )
            }
            other if !other.starts_with("--") && prompt.is_none() => {
                prompt = Some(other.to_owned())
            }
            other => return Err(format!("unknown flag {other}")),
        }
    }
    let data_dir = data_dir
        .or_else(|| std::env::var_os("BRIDGE_DATA_DIR").map(PathBuf::from))
        .ok_or("--data-dir is required (or set BRIDGE_DATA_DIR)")?;
    if !json {
        return Err("exec emits JSON lines; pass --json explicitly".into());
    }
    match (&method, &prompt) {
        (None, None) => return Err("pass a prompt, or --method for a single call".into()),
        (None, Some(_)) if harness.is_none() => {
            return Err(
                "prompt mode needs --harness naming an available structured adapter \
                 (see health/health adapters)"
                    .into(),
            )
        }
        _ => {}
    }
    Ok(ExecFlags { data_dir, harness, model, timeout, method, params, prompt })
}

/// The whole command's time budget, shared by every call.
struct Budget {
    deadline: Instant,
}

impl Budget {
    fn new(timeout: Duration) -> Budget {
        Budget { deadline: Instant::now() + timeout }
    }

    fn deadline(&self) -> Instant {
        self.deadline
    }

    fn remaining(&self) -> Result<Duration, String> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("the --timeout budget is exhausted".into());
        }
        Ok(remaining)
    }

    fn call(
        &self,
        client: &DaemonClient,
        method: MethodName,
        params: Option<Value>,
    ) -> Result<Value, String> {
        client
            .call_with_timeout(method, params, self.remaining()?)
            .map_err(|error| format!("{}: {error}", method.as_str()))
    }
}

/// The daemon this exec talks to: an existing one, or one it hosts itself for
/// the duration of the command.
enum ExecHost {
    Attached,
    SelfHosted { daemon: std::sync::Arc<bridged::Daemon>, accept_loop: std::thread::JoinHandle<()> },
}

impl ExecHost {
    fn finish(self) {
        if let ExecHost::SelfHosted { daemon, accept_loop } = self {
            daemon
                .state
                .shutting_down
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let _ = accept_loop.join();
            // Graceful shutdown stops any adapter this one-shot started.
            daemon.shutdown(bridged::DEFAULT_DRAIN_TIMEOUT);
        }
    }
}

fn connect_or_host(data_dir: &Path, budget: &Budget) -> Result<(ExecHost, DaemonClient), String> {
    // A running daemon owns the directory: attach.
    if let Ok(endpoint) = Endpoint::for_data_dir(data_dir) {
        if let Ok(client) = DaemonClient::connect_with_timeout(
            &endpoint,
            budget.remaining()?.min(bridge_client::DEFAULT_CONNECT_TIMEOUT),
        ) {
            return Ok((ExecHost::Attached, client));
        }
    }
    // Otherwise host the runtime one-shot. The lease arbitrates the race:
    // losing it means someone else owns the directory — report who.
    let (daemon, listener) = bridged::Daemon::start(bridged::DaemonConfig {
        data_dir: data_dir.to_path_buf(),
        socket_path: None,
        health_addr: None,
        browser_extension_path: default_browser_extension(),
        handshake_timeout: bridged::DEFAULT_HANDSHAKE_TIMEOUT,
    })
    .map_err(|error| error.to_string())?;
    let daemon = std::sync::Arc::new(daemon);
    let serve_daemon = daemon.clone();
    let accept_loop = std::thread::Builder::new()
        .name("bridge-exec-daemon".into())
        .spawn(move || {
            let _ = bridged::serve(&serve_daemon, listener);
        })
        .expect("exec daemon thread spawns");
    let endpoint = Endpoint::for_data_dir(data_dir).map_err(|error| error.to_string())?;
    let client = DaemonClient::connect_with_timeout(
        &endpoint,
        budget.remaining()?.min(bridge_client::DEFAULT_CONNECT_TIMEOUT),
    )
    .map_err(|error| error.to_string())?;
    Ok((ExecHost::SelfHosted { daemon, accept_loop }, client))
}

fn default_browser_extension() -> PathBuf {
    std::env::var_os("BRIDGE_BROWSER_EXTENSION")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../browser-extension")
        })
}

fn emit(line: &Value) {
    println!("{line}");
}

fn exec(args: &[String]) -> ExitCode {
    let flags = match parse_exec_flags(args) {
        Ok(flags) => flags,
        Err(message) => {
            eprintln!("bridge exec: {message}");
            return ExitCode::from(2);
        }
    };
    let budget = Budget::new(flags.timeout);
    let (host, client) = match connect_or_host(&flags.data_dir, &budget) {
        Ok(connected) => connected,
        Err(message) => {
            eprintln!("bridge exec: {message}");
            return ExitCode::FAILURE;
        }
    };
    let outcome = if let Some(method) = &flags.method {
        exec_method(&client, method, flags.params.clone(), &budget)
    } else {
        exec_turn(&client, &flags, &budget)
    };
    drop(client);
    host.finish();
    match outcome {
        Ok(code) => code,
        Err(message) => {
            eprintln!("bridge exec: {message}");
            ExitCode::FAILURE
        }
    }
}

/// One method call, result on stdout.
fn exec_method(
    client: &DaemonClient,
    method: &str,
    params: Option<Value>,
    budget: &Budget,
) -> Result<ExitCode, String> {
    match client.call_raw(method, params, budget.remaining()?) {
        Ok(result) => {
            emit(&json!({"type": "result", "method": method, "result": result}));
            Ok(ExitCode::SUCCESS)
        }
        Err(ClientError::Rpc(error)) => {
            emit(&json!({
                "type": "error",
                "method": method,
                "code": error.code,
                "message": error.message,
                "data": error.data,
            }));
            Ok(ExitCode::FAILURE)
        }
        Err(error) => Err(error.to_string()),
    }
}

/// Whether a `turn.completed` event itself reports the turn as failed.
fn completed_turn_failed(payload: &Value) -> bool {
    payload["status"].as_str() == Some("failed")
}

/// Whether an event is a trailing turn failure (providers may report failure
/// after completing the turn).
fn is_failure_event(payload: &Value) -> bool {
    payload["kind"].as_str() == Some("error") && payload["status"].as_str() == Some("failed")
}

/// Codex-exec style: create a chat, run one turn, stream its durable events
/// as JSON lines until the turn completes.
fn exec_turn(client: &DaemonClient, flags: &ExecFlags, budget: &Budget) -> Result<ExitCode, String> {
    let prompt = flags.prompt.as_deref().expect("checked in parse");
    let harness = flags.harness.as_deref().expect("checked in parse");

    // The harness must be an available structured adapter — anything else
    // fails after creating a stray session; fail before instead.
    let health = budget.call(client, MethodName::Health, None)?;
    let adapter = health["adapters"]
        .as_array()
        .and_then(|adapters| {
            adapters
                .iter()
                .find(|adapter| adapter["id"].as_str() == Some(harness))
        })
        .cloned();
    match adapter {
        Some(adapter) if adapter["available"].as_bool() == Some(true) => {}
        other => {
            let available: Vec<String> = health["adapters"]
                .as_array()
                .map(|adapters| {
                    adapters
                        .iter()
                        .filter(|adapter| adapter["available"].as_bool() == Some(true))
                        .filter_map(|adapter| adapter["id"].as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            return Err(format!(
                "harness {harness} is {} — available structured adapters: [{}]",
                if other.is_none() { "not a structured adapter" } else { "not available" },
                available.join(", "),
            ));
        }
    }

    let mut create = json!({"harness": harness, "title": "bridge exec"});
    if let Some(model) = &flags.model {
        create["model"] = json!(model);
    }
    let created = budget.call(client, MethodName::CreateChatId, Some(create))?;
    let created: bridge_protocol::messages::CreateChatIdResult = serde_json::from_value(created)
        .map_err(|error| format!("invalid chat creation result: {error}"))?;
    let session_id = created.session_id;
    emit(&json!({"type": "session", "sessionId": session_id, "harness": harness}));

    // Subscribe from cursor 0 before starting, so nothing between start and
    // first poll can be missed; the stream discards replayed duplicates.
    let mut events = SessionEventStream::new(client, session_id.clone(), 0)
        .map_err(|error| error.to_string())?;
    budget.call(client, MethodName::StartChat, Some(json!({"sessionId": session_id})))?;
    budget.call(
        client,
        MethodName::SendTurn,
        Some(json!({"sessionId": session_id, "text": prompt})),
    )?;

    loop {
        match events.next(budget.deadline()).map_err(|error| error.to_string())? {
            None => {
                emit(&json!({
                    "type": "exec.timeout",
                    "sessionId": session_id,
                    "cursor": events.cursor(),
                }));
                // Best-effort domain interrupt for the still-running turn; a
                // failure to interrupt is reported, not swallowed.
                if let Err(error) = client.call_with_timeout(
                    MethodName::InterruptTurn,
                    Some(json!({"sessionId": session_id})),
                    Duration::from_secs(10),
                ) {
                    eprintln!("bridge exec: interrupt after timeout failed: {error}");
                }
                return Ok(ExitCode::from(3));
            }
            Some(event) => {
                emit(&event.payload);
                if is_failure_event(&event.payload) {
                    return finish_turn(&session_id, &events, "failed");
                }
                if event.payload["kind"].as_str() == Some("turn.completed") {
                    if completed_turn_failed(&event.payload) {
                        return finish_turn(&session_id, &events, "failed");
                    }
                    // Some providers complete the turn, then report its
                    // failure: drain a short grace window, then let the
                    // session's own status settle it.
                    let grace = Instant::now() + FAILURE_GRACE;
                    while let Some(trailing) =
                        events.next(grace).map_err(|error| error.to_string())?
                    {
                        emit(&trailing.payload);
                        if is_failure_event(&trailing.payload) {
                            return finish_turn(&session_id, &events, "failed");
                        }
                    }
                    let state = budget.call(client, MethodName::GetState, None)?;
                    let failed = state["sessions"]
                        .as_array()
                        .and_then(|sessions| {
                            sessions
                                .iter()
                                .find(|session| session["id"].as_str() == Some(&session_id))
                        })
                        .is_some_and(|session| session["status"].as_str() == Some("failed"));
                    return finish_turn(
                        &session_id,
                        &events,
                        if failed { "failed" } else { "completed" },
                    );
                }
            }
        }
    }
}

/// Emit the summary and translate the outcome to an exit code. The session is
/// deliberately left alone: an attached daemon owns it (it appears in the app
/// like any chat), and a self-hosted run stops adapters on shutdown — the
/// interactive `stop_session` checkpoint flow is not a one-shot's business.
fn finish_turn(
    session_id: &str,
    events: &SessionEventStream,
    status: &str,
) -> Result<ExitCode, String> {
    emit(&json!({
        "type": "exec.completed",
        "sessionId": session_id,
        "status": status,
        "cursor": events.cursor(),
    }));
    Ok(if status == "failed" { ExitCode::FAILURE } else { ExitCode::SUCCESS })
}

// --- learning run (unchanged behavior, re-homed off the Tauri crate) ----------

fn learning_run(args: &[String]) -> ExitCode {
    let value_after = |flag: &str| {
        args.iter()
            .position(|value| value == flag)
            .and_then(|index| args.get(index + 1))
            .cloned()
    };
    let Some(database) = value_after("--database")
        .or_else(|| std::env::var("BRIDGE_DB").ok())
        .map(PathBuf::from)
    else {
        eprintln!(
            "usage: bridge learning run --database <bridge.db> --trigger \
             <manual|in-app|codex:ID|claude:ID|opencode:ID> [--credential-ref <reference>]"
        );
        return ExitCode::from(2);
    };
    let trigger = value_after("--trigger").unwrap_or_else(|| "manual".into());
    let credential_ref = value_after("--credential-ref");
    match bridge_core::learning_job::run_database(&database, &trigger, credential_ref.as_deref()) {
        Ok(run) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&run).expect("learning run should serialize")
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Bridge learning run failed: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fleet_summary_counts_live_working_and_waiting_sessions() {
        let state = json!({ "sessions": [
            { "id": "a", "label": "Chat", "title": "Fix login", "harness": "claude", "status": "working", "endedAt": null },
            { "id": "b", "label": "Research · strong", "harness": "codex", "status": "waiting", "parentSessionId": "a" },
            { "id": "c", "label": "Old", "harness": "codex", "status": "working", "endedAt": "2026-10-01T00:00:00Z" },
            { "id": "d", "label": "Idle", "harness": "claude", "status": "ready" },
            { "id": "e", "label": "Boot", "harness": "opencode", "status": "starting" },
        ]});
        let summary = summarize(&state);
        assert_eq!(summary["running"], true);
        assert_eq!(summary["working"], 2, "a and e; c has ended");
        assert_eq!(summary["waiting"], 1);
        let ids: Vec<_> = summary["sessions"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["a", "b", "e"]);
        assert_eq!(summary["sessions"][0]["label"], "Fix login", "a title wins over the label");
        assert_eq!(summarize(&json!({})), json!({ "running": true, "working": 0, "waiting": 0, "sessions": [] }));
        assert_eq!(stopped_summary()["running"], false);
    }

    #[test]
    fn approval_actions_map_to_wire_decisions() {
        assert_eq!(approval_decision("allow-once"), Some("accept"));
        assert_eq!(approval_decision("allow-session"), Some("acceptForSession"));
        assert_eq!(approval_decision("decline"), Some("decline"));
        assert_eq!(approval_decision("approve"), None);
    }

    #[test]
    fn surface_flags_parse_and_default_the_data_dir() {
        let args = |list: &[&str]| list.iter().map(|value| value.to_string()).collect::<Vec<_>>();
        let flags = parse_surface_flags(&args(&["--json", "--watch", "--data-dir", "/tmp/b"]), true).unwrap();
        assert_eq!(flags, SurfaceFlags { data_dir: PathBuf::from("/tmp/b"), watch: true, positional: vec![] });
        assert!(parse_surface_flags(&args(&["--watch", "--json", "--data-dir", "/tmp/b"]), false).unwrap_err().contains("--watch"));
        assert!(parse_surface_flags(&args(&["--data-dir", "/tmp/b"]), true).unwrap_err().contains("--json"));
        let approval = parse_surface_flags(&args(&["allow-once", "s1", "42", "--data-dir", "/tmp/b"]), false).unwrap();
        assert_eq!(approval.positional, ["allow-once", "s1", "42"]);
    }

    #[test]
    fn turn_failures_are_recognized_in_both_provider_shapes() {
        // Codex: the completion itself carries the failed status.
        assert!(completed_turn_failed(&json!({
            "kind": "turn.completed", "status": "failed",
        })));
        assert!(!completed_turn_failed(&json!({
            "kind": "turn.completed", "status": "completed",
        })));
        // Claude: completion first, failure as a trailing error event.
        assert!(is_failure_event(&json!({"kind": "error", "status": "failed"})));
        assert!(!is_failure_event(&json!({"kind": "error", "status": "recovered"})));
        assert!(!is_failure_event(&json!({"kind": "message.completed", "status": "failed"})));
    }

    #[test]
    fn prompt_mode_requires_a_harness_and_json() {
        let parse = |args: &[&str]| {
            parse_exec_flags(&args.iter().map(|value| value.to_string()).collect::<Vec<_>>())
        };
        let error = parse(&["--json", "--data-dir", "/tmp/x", "do things"]).unwrap_err();
        assert!(error.contains("--harness"), "{error}");
        let error = parse(&["--data-dir", "/tmp/x", "--method", "health/health"]).unwrap_err();
        assert!(error.contains("--json"), "{error}");
        assert!(parse(&["--json", "--data-dir", "/tmp/x", "--method", "health/health"]).is_ok());
        assert!(parse(&["--json", "--data-dir", "/tmp/x", "--harness", "codex", "hi"]).is_ok());
    }
}

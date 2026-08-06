//! The `bridge` CLI.
//!
//! ```text
//! bridge exec --json [flags] "<prompt>"        run one turn, stream JSONL events
//! bridge exec --json [flags] --method <m> [--params <json>]
//!                                              call one protocol method
//! bridge learning run --database <bridge.db> --trigger <...>
//! ```
//!
//! `exec` is the CI one-shot: it attaches to a running daemon when one owns
//! the data directory, and otherwise hosts the runtime itself for exactly the
//! duration of the command — CI never keeps a user daemon alive. Every event
//! and result is a JSON line on stdout; diagnostics go to stderr.

use bridge_client::{DaemonClient, Endpoint, SessionEventStream};
use bridge_protocol::MethodName;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("exec") => exec(&args[1..]),
        Some("learning") if args.get(1).map(String::as_str) == Some("run") => {
            learning_run(&args[2..])
        }
        _ => {
            eprintln!(
                "usage:\n  bridge exec --json [--data-dir <dir>] [--harness <id>] [--model <id>] \
                 [--timeout <secs>] (\"<prompt>\" | --method <name> [--params <json>])\n  \
                 bridge learning run --database <bridge.db> --trigger \
                 <manual|in-app|codex:ID|claude:ID|opencode:ID> [--credential-ref <reference>]"
            );
            ExitCode::from(2)
        }
    }
}

// --- exec ---------------------------------------------------------------------

struct ExecFlags {
    data_dir: PathBuf,
    harness: String,
    model: Option<String>,
    timeout: Duration,
    method: Option<String>,
    params: Option<Value>,
    prompt: Option<String>,
}

fn parse_exec_flags(args: &[String]) -> Result<ExecFlags, String> {
    let mut data_dir: Option<PathBuf> = None;
    let mut harness = "shell".to_owned();
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
            "--harness" => harness = value("--harness")?,
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
    if method.is_none() && prompt.is_none() {
        return Err("pass a prompt, or --method for a single call".into());
    }
    Ok(ExecFlags { data_dir, harness, model, timeout, method, params, prompt })
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
            daemon.shutdown(bridged::DEFAULT_DRAIN_TIMEOUT);
        }
    }
}

fn connect_or_host(data_dir: &Path) -> Result<(ExecHost, DaemonClient), String> {
    // A running daemon owns the directory: attach.
    if let Ok(endpoint) = Endpoint::for_data_dir(data_dir) {
        if let Ok(client) = DaemonClient::connect(&endpoint) {
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
    let client = DaemonClient::connect(&endpoint).map_err(|error| error.to_string())?;
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
    let (host, client) = match connect_or_host(&flags.data_dir) {
        Ok(connected) => connected,
        Err(message) => {
            eprintln!("bridge exec: {message}");
            return ExitCode::FAILURE;
        }
    };
    let outcome = if let Some(method) = &flags.method {
        exec_method(&client, method, flags.params.clone())
    } else {
        exec_turn(&client, &flags)
    };
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
fn exec_method(client: &DaemonClient, method: &str, params: Option<Value>) -> Result<ExitCode, String> {
    match client.call_raw(method, params) {
        Ok(result) => {
            emit(&json!({"type": "result", "method": method, "result": result}));
            Ok(ExitCode::SUCCESS)
        }
        Err(bridge_client::ClientError::Rpc(error)) => {
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

/// Codex-exec style: create a chat, run one turn, stream its durable events
/// as JSON lines until the turn completes.
fn exec_turn(client: &DaemonClient, flags: &ExecFlags) -> Result<ExitCode, String> {
    let prompt = flags.prompt.as_deref().expect("checked in parse");
    let mut create = json!({"harness": flags.harness, "title": "bridge exec"});
    if let Some(model) = &flags.model {
        create["model"] = json!(model);
    }
    let state = client
        .call(MethodName::CreateChat, Some(create))
        .map_err(|error| error.to_string())?;
    let session_id = state["sessions"]
        .as_array()
        .and_then(|sessions| {
            sessions
                .iter()
                .max_by_key(|session| session["startedAt"].as_str().map(str::to_owned))
        })
        .and_then(|session| session["id"].as_str())
        .ok_or("create_chat returned no session")?
        .to_owned();
    emit(&json!({"type": "session", "sessionId": session_id, "harness": flags.harness}));

    // Subscribe from cursor 0 before starting, so nothing between start and
    // first poll can be missed; the stream discards replayed duplicates.
    let mut events = SessionEventStream::new(client, session_id.clone(), 0)
        .map_err(|error| error.to_string())?;
    client
        .call(MethodName::StartChat, Some(json!({"sessionId": session_id})))
        .map_err(|error| format!("start_chat: {error}"))?;
    client
        .call(
            MethodName::SendTurn,
            Some(json!({"sessionId": session_id, "text": prompt})),
        )
        .map_err(|error| format!("send_turn: {error}"))?;

    let deadline = Instant::now() + flags.timeout;
    loop {
        match events.next(deadline).map_err(|error| error.to_string())? {
            None => {
                emit(&json!({
                    "type": "exec.timeout",
                    "sessionId": session_id,
                    "cursor": events.cursor(),
                }));
                let _ = client.call(
                    MethodName::StopSession,
                    Some(json!({"sessionId": session_id})),
                );
                return Ok(ExitCode::from(3));
            }
            Some(event) => {
                emit(&event.payload);
                let kind = event.payload["kind"].as_str().unwrap_or_default();
                let failed = kind == "error"
                    && event.payload["status"].as_str() == Some("failed");
                if failed || kind == "turn.completed" {
                    let _ = client.call(
                        MethodName::StopSession,
                        Some(json!({"sessionId": session_id})),
                    );
                    emit(&json!({
                        "type": "exec.completed",
                        "sessionId": session_id,
                        "status": if failed { "failed" } else { "completed" },
                        "cursor": events.cursor(),
                    }));
                    return Ok(if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS });
                }
            }
        }
    }
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

//! Persistent terminal workspaces. Rust owns PTYs; the same xterm headless
//! engine used by Orca owns VT checkpoints and the ordered output journal.
use crate::{events::CoreEvent, BridgeCore, BridgeError, RuntimeSession};
use base64::Engine;
use bridge_protocol::messages::{
    CreateTerminalParams, TerminalFrame, TerminalRecord, TerminalSnapshot, TerminalWorkspace,
};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{mpsc, Arc};
use std::time::Duration;

pub(crate) struct StateSidecar {
    child: Child,
    input: ChildStdin,
    output: mpsc::Receiver<Result<Value, String>>,
}

impl Drop for StateSidecar {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn script_path() -> Result<PathBuf, BridgeError> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("BRIDGE_TERMINAL_STATE_SIDECAR") {
        candidates.push(PathBuf::from(path));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("../Resources/sidecar/terminal-state/index.mjs"));
            candidates.push(dir.join("sidecar/terminal-state/index.mjs"));
        }
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sidecar/terminal-state/index.mjs"),
    );
    candidates.into_iter().find(|p| p.is_file()).ok_or_else(|| {
        BridgeError::Invalid("Terminal history runtime is missing. Reinstall Bridge.".into())
    })
}

impl StateSidecar {
    fn start(core: &BridgeCore) -> Result<Self, BridgeError> {
        let node = crate::binary::resolve("node").ok_or_else(|| BridgeError::Invalid("Node.js is required for persistent terminals. Install Node.js and reopen Mission Control.".into()))?;
        let root = core
            .database_path
            .parent()
            .ok_or_else(|| {
                BridgeError::Invalid("Terminal storage directory is unavailable".into())
            })?
            .join("terminal-state");
        let mut child = Command::new(node)
            .arg(script_path()?)
            .arg(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let input = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, output) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let result = line
                    .map_err(|e| e.to_string())
                    .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()));
                if tx.send(result).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input,
            output,
        })
    }
    fn call(&mut self, request: Value) -> Result<Value, BridgeError> {
        serde_json::to_writer(&mut self.input, &request)
            .map_err(|e| BridgeError::Invalid(e.to_string()))?;
        self.input.write_all(b"\n")?;
        self.input.flush()?;
        let response = self
            .output
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| {
                let _ = self.child.kill();
                BridgeError::Invalid("Terminal history runtime stopped responding".into())
            })?
            .map_err(BridgeError::Invalid)?;
        if let Some(error) = response.get("error").and_then(Value::as_str) {
            return Err(BridgeError::Invalid(error.into()));
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }
}

fn state_call<T: DeserializeOwned>(core: &BridgeCore, request: Value) -> Result<T, BridgeError> {
    let mut slot = core.terminal_state.lock().unwrap();
    if slot.is_none() {
        *slot = Some(StateSidecar::start(core)?);
    }
    let result = slot.as_mut().unwrap().call(request);
    if slot
        .as_mut()
        .unwrap()
        .child
        .try_wait()
        .ok()
        .flatten()
        .is_some()
    {
        slot.take();
    }
    serde_json::from_value(result?)
        .map_err(|e| BridgeError::Invalid(format!("Terminal state: {e}")))
}

fn key(workspace: &str, terminal: &str) -> String {
    format!("terminal:{workspace}:{terminal}")
}
fn validate_id(id: &str) -> Result<(), BridgeError> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
    {
        return Err(BridgeError::Invalid("Invalid terminal identity".into()));
    }
    Ok(())
}
fn is_live(core: &BridgeCore, workspace: &str, terminal: &str) -> bool {
    core.runtimes
        .lock()
        .unwrap()
        .contains_key(&key(workspace, terminal))
}

pub fn snapshot(
    core: &Arc<BridgeCore>,
    workspace: &str,
    terminal: &str,
) -> Result<TerminalSnapshot, BridgeError> {
    let operation = core.workspace_operation(workspace);
    let _operation = operation.lock().unwrap();
    snapshot_inner(core, workspace, terminal)
}

// The workspace operation also covers creation's checkpoint-before-spawn
// interval, which must never be mistaken for a dead process during recovery.
fn snapshot_inner(
    core: &Arc<BridgeCore>,
    workspace: &str,
    terminal: &str,
) -> Result<TerminalSnapshot, BridgeError> {
    core.workspace_path(workspace)?;
    validate_id(terminal)?;
    let mut snapshot: TerminalSnapshot = state_call(
        core,
        json!({"op":"snapshot", "key":key(workspace, terminal)}),
    )?;
    if snapshot.record.status == "running" && !is_live(core, workspace, terminal) {
        let _: TerminalRecord = state_call(
            core,
            json!({"op":"update", "key":key(workspace, terminal), "changes":{"generation":snapshot.record.generation,"status":"exited"}}),
        )?;
        snapshot = state_call(
            core,
            json!({"op":"snapshot", "key":key(workspace, terminal)}),
        )?;
    }
    Ok(snapshot)
}

pub fn workspace(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
) -> Result<TerminalWorkspace, BridgeError> {
    let operation = core.workspace_operation(workspace_id);
    let _operation = operation.lock().unwrap();
    core.workspace_path(workspace_id)?;
    let mut terminals: Vec<TerminalRecord> =
        state_call(core, json!({"op":"list", "workspaceId":workspace_id}))?;
    for terminal in &mut terminals {
        if terminal.status == "running" && !is_live(core, workspace_id, &terminal.terminal_id) {
            *terminal = state_call(
                core,
                json!({"op":"update", "key":key(workspace_id, &terminal.terminal_id), "changes":{"generation":terminal.generation,"status":"exited"}}),
            )?;
        }
    }
    let layout = state_call(core, json!({"op":"layout", "workspaceId":workspace_id}))?;
    Ok(TerminalWorkspace { terminals, layout })
}

pub fn save_layout(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
    layout: Value,
) -> Result<(), BridgeError> {
    core.workspace_path(workspace_id)?;
    if layout.to_string().len() > 256 * 1024 {
        return Err(BridgeError::Invalid("Terminal layout is too large".into()));
    }
    let _: Value = state_call(
        core,
        json!({"op":"layout", "workspaceId":workspace_id, "value":layout}),
    )?;
    Ok(())
}

pub fn rename(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
    terminal_id: &str,
    title: &str,
) -> Result<TerminalRecord, BridgeError> {
    core.workspace_path(workspace_id)?;
    validate_id(terminal_id)?;
    if title.trim().is_empty() || title.chars().count() > 120 {
        return Err(BridgeError::Invalid(
            "Use a terminal title between 1 and 120 characters".into(),
        ));
    }
    state_call(
        core,
        json!({"op":"update", "key":key(workspace_id, terminal_id), "changes":{"title":title.trim()}}),
    )
}

pub fn create(
    core: &Arc<BridgeCore>,
    params: &CreateTerminalParams,
) -> Result<TerminalRecord, BridgeError> {
    let workspace_id = &params.workspace_id;
    let terminal_id = &params.terminal_id;
    validate_id(terminal_id)?;
    let workspace_path = core.workspace_path(workspace_id)?;
    let runtime_id = key(workspace_id, terminal_id);
    let operation = core.workspace_operation(workspace_id);
    let _operation = operation.lock().unwrap();
    let _lifecycle = core.claim_session_lifecycle(&runtime_id, "terminal create")?;
    if is_live(core, workspace_id, terminal_id) {
        return Ok(snapshot_inner(core, workspace_id, terminal_id)?.record);
    }
    let previous = match snapshot_inner(core, workspace_id, terminal_id) {
        Ok(snapshot) => Some(snapshot.record),
        Err(BridgeError::Invalid(message)) if message.contains("ENOENT") => None,
        Err(error) => return Err(error),
    };
    if let Some(record) = &previous {
        if !params.restart {
            return Ok(record.clone());
        }
    }
    let agent_id = if params.restart {
        previous
            .as_ref()
            .and_then(|r| r.agent_id.clone())
            .or_else(|| params.agent_id.clone())
    } else {
        params.agent_id.clone()
    };
    let cwd = params
        .cwd
        .as_deref()
        .or_else(|| previous.as_ref().map(|r| r.cwd.as_str()))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(workspace_path));
    let cwd = cwd
        .canonicalize()
        .map_err(|e| BridgeError::Invalid(format!("Cannot open terminal directory: {e}")))?;
    if !cwd.is_dir() {
        return Err(BridgeError::Invalid(
            "Terminal directory is not a folder".into(),
        ));
    }
    let executable = match agent_id.as_deref() {
        None => PathBuf::from(super::api::login_shell()),
        Some(id) => crate::managed_agents::interactive_executable(id)?,
    };
    let mut command = CommandBuilder::new(executable);
    if agent_id.is_none() {
        command.arg("-l");
    }
    command.cwd(&cwd);
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command.env("TERM_PROGRAM", "Bridge");
    command.env("BRIDGE_WORKSPACE_ID", workspace_id);
    let record = TerminalRecord {
        workspace_id: workspace_id.clone(),
        terminal_id: terminal_id.clone(),
        generation: uuid::Uuid::new_v4().to_string(),
        title: previous
            .as_ref()
            .map(|r| r.title.clone())
            .unwrap_or_else(|| agent_id.clone().unwrap_or("Shell".into())),
        cwd: cwd.to_string_lossy().into_owned(),
        agent_id,
        status: "running".into(),
        rows: 32,
        cols: 120,
        created_at: chrono::Utc::now().to_rfc3339(),
        exit_code: None,
        history_truncated: false,
    };
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: record.rows,
            cols: record.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| BridgeError::Pty(e.to_string()))?;
    // Acquire every fallible PTY handle before publishing a running record or
    // spawning a child. A failed reader/writer must not orphan a process.
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| BridgeError::Pty(e.to_string()))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| BridgeError::Pty(e.to_string()))?;
    let _: TerminalRecord = state_call(
        core,
        json!({"op":"create", "key":runtime_id, "record":record}),
    )?;
    let child = match pair.slave.spawn_command(command) {
        Ok(child) => child,
        Err(error) => {
            let _: Result<TerminalRecord, _> = state_call(
                core,
                json!({"op":"update", "key":runtime_id, "changes":{"status":"exited"}}),
            );
            return Err(BridgeError::Pty(error.to_string()));
        }
    };
    drop(pair.slave);
    let epoch = super::api::TERMINAL_EPOCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    core.runtimes.lock().unwrap().insert(
        runtime_id.clone(),
        RuntimeSession {
            writer,
            master: pair.master,
            child,
            epoch,
        },
    );
    let owned = record.clone();
    let core = core.clone();
    std::thread::spawn(move || {
        let mut bytes = [0; 8192];
        let mut sequence = 0;
        loop {
            let count = match reader.read(&mut bytes) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let response: Value = match state_call(
                &core,
                json!({"op":"write", "key":runtime_id, "generation":owned.generation, "bytes":base64::engine::general_purpose::STANDARD.encode(&bytes[..count])}),
            ) {
                Ok(response) => response,
                Err(error) => {
                    eprintln!("terminal history failed: {error}");
                    break;
                }
            };
            if response.is_null() {
                return;
            }
            sequence = response["sequence"].as_u64().unwrap_or(sequence);
            let data = response["data"].as_str().unwrap_or("").to_owned();
            core.events.publish(CoreEvent::TerminalFrame(TerminalFrame {
                workspace_id: owned.workspace_id.clone(),
                terminal_id: owned.terminal_id.clone(),
                generation: owned.generation.clone(),
                sequence,
                data: data.clone(),
                rows: None,
                cols: None,
                status: None,
            }));
            core.events.publish(CoreEvent::SessionOutput {
                session_id: owned.workspace_id.clone(),
                terminal_id: owned.terminal_id.clone(),
                data,
            });
        }
        let mut runtimes = core.runtimes.lock().unwrap();
        let exit_code = match runtimes.get(&runtime_id) {
            Some(runtime) if runtime.epoch != epoch => return,
            Some(_) => {
                let mut runtime = runtimes.remove(&runtime_id).unwrap();
                let exit = runtime.child.try_wait().ok().flatten();
                if exit.is_none() {
                    let _ = runtime.child.kill();
                }
                exit.map(|status| status.exit_code())
            }
            None => None,
        };
        drop(runtimes);
        let _: Result<TerminalRecord, _> = state_call(
            &core,
            json!({"op":"update", "key":runtime_id, "changes":{"generation":owned.generation,"status":"exited","exitCode":exit_code}}),
        );
        if let Ok(final_state) =
            state_call::<Value>(&core, json!({"op":"describe", "key":runtime_id}))
        {
            sequence = final_state["sequence"].as_u64().unwrap_or(sequence + 1);
        } else {
            sequence += 1;
        }
        core.events.publish(CoreEvent::TerminalFrame(TerminalFrame {
            workspace_id: owned.workspace_id.clone(),
            terminal_id: owned.terminal_id.clone(),
            generation: owned.generation,
            sequence,
            data: String::new(),
            rows: None,
            cols: None,
            status: Some("exited".into()),
        }));
        core.events.publish(CoreEvent::TerminalExited {
            session_id: owned.workspace_id,
            terminal_id: owned.terminal_id,
        });
    });
    Ok(record)
}

pub fn resized(
    core: &Arc<BridgeCore>,
    workspace: &str,
    terminal: &str,
    rows: u16,
    cols: u16,
) -> Result<(), BridgeError> {
    if !(2..=500).contains(&rows) || !(2..=1000).contains(&cols) {
        return Err(BridgeError::Invalid(
            "Terminal dimensions are out of range".into(),
        ));
    }
    let mut runtimes = core.runtimes.lock().unwrap();
    if let Some(runtime) = runtimes.get_mut(&key(workspace, terminal)) {
        if workspace == "provider-login" {
            return runtime
                .master
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| BridgeError::Pty(e.to_string()));
        }
        // Keep the PTY resize and its journal record ordered against writes.
        let mut state = core.terminal_state.lock().unwrap();
        if state.is_none() {
            *state = Some(StateSidecar::start(core)?);
        }
        if let Some(sidecar) = state.as_mut() {
            let snapshot =
                sidecar.call(json!({"op":"describe", "key":key(workspace, terminal)}))?;
            let generation = snapshot["record"]["generation"].as_str().unwrap_or("");
            runtime
                .master
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| BridgeError::Pty(e.to_string()))?;
            let result = sidecar.call(json!({"op":"resize", "key":key(workspace, terminal), "generation":generation, "rows":rows, "cols":cols}))?;
            core.events.publish(CoreEvent::TerminalFrame(TerminalFrame {
                workspace_id: workspace.into(),
                terminal_id: terminal.into(),
                generation: generation.into(),
                sequence: result["sequence"].as_u64().unwrap_or(0),
                data: String::new(),
                rows: Some(rows),
                cols: Some(cols),
                status: None,
            }));
        }
    }
    Ok(())
}

pub fn closed(core: &Arc<BridgeCore>, workspace: &str, terminal: &str) -> Result<(), BridgeError> {
    core.workspace_path(workspace)?;
    validate_id(terminal)?;
    match state_call::<TerminalRecord>(
        core,
        json!({"op":"update", "key":key(workspace, terminal), "changes":{"status":"closed"}}),
    ) {
        Ok(_) => {}
        Err(BridgeError::Invalid(message)) if message.contains("ENOENT") => {}
        Err(error) => return Err(error),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &std::path::Path) -> Arc<BridgeCore> {
        let core = Arc::new(BridgeCore::for_tests(root));
        {
            let db = core.db.lock().unwrap();
            db.execute(
                "INSERT INTO projects(id,name,path,created_at) VALUES('p','P','/tmp','now')",
                [],
            )
            .unwrap();
            db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Task','main',?1,'ready','now')", rusqlite::params![root.to_string_lossy()]).unwrap();
        }
        core
    }
    fn params() -> CreateTerminalParams {
        CreateTerminalParams {
            workspace_id: "w".into(),
            terminal_id: "pane".into(),
            agent_id: None,
            cwd: None,
            restart: false,
        }
    }
    fn wait_for(
        core: &Arc<BridgeCore>,
        predicate: impl Fn(&TerminalSnapshot) -> bool,
    ) -> TerminalSnapshot {
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        loop {
            let value = snapshot(core, "w", "pane").unwrap();
            if predicate(&value) {
                return value;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "terminal did not reach expected state: {:?}",
                value.record
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    #[test]
    fn terminal_workspace_records_unattended_output_and_reattaches_without_respawning() {
        let root = tempfile::tempdir().unwrap();
        let core = fixture(root.path());
        let original = create(&core, &params()).unwrap();
        let epoch = core.runtimes.lock().unwrap()["terminal:w:pane"].epoch;
        crate::api::write_terminal(&core, "w", "pane", "printf 'BRIDGE_%s\\n' RECOVERED\n")
            .unwrap();
        wait_for(&core, |s| s.ansi.contains("BRIDGE_RECOVERED"));
        let attached = create(&core, &params()).unwrap();
        assert_eq!(original.generation, attached.generation);
        assert_eq!(
            epoch,
            core.runtimes.lock().unwrap()["terminal:w:pane"].epoch
        );
        resized(&core, "w", "pane", 24, 90).unwrap();
        assert_eq!(snapshot(&core, "w", "pane").unwrap().record.cols, 90);
        save_layout(&core, "w", json!({"version":1,"tabs":[]})).unwrap();
        // A renderer disconnect does not own the runtime. Restart only the
        // history helper to exercise the disk checkpoint plus journal path.
        core.terminal_state.lock().unwrap().take();
        assert!(snapshot(&core, "w", "pane")
            .unwrap()
            .ansi
            .contains("BRIDGE_RECOVERED"));
        crate::api::write_terminal(&core, "w", "pane", "exit\n").unwrap();
        wait_for(&core, |s| s.record.status == "exited");
        let restored_core = fixture(root.path());
        let restored = create(&restored_core, &params()).unwrap();
        assert_eq!(restored.generation, original.generation);
        assert_eq!(restored.status, "exited");
        assert!(restored_core.runtimes.lock().unwrap().is_empty());
        assert!(snapshot(&restored_core, "w", "pane")
            .unwrap()
            .ansi
            .contains("BRIDGE_RECOVERED"));
        assert_eq!(workspace(&restored_core, "w").unwrap().layout["version"], 1);
        let restarted = create(
            &restored_core,
            &CreateTerminalParams {
                restart: true,
                ..params()
            },
        )
        .unwrap();
        assert_ne!(restarted.generation, original.generation);
        crate::api::close_terminal(&restored_core, "w", "pane").unwrap();
        assert!(workspace(&restored_core, "w").unwrap().terminals.is_empty());
    }
    #[test]
    fn terminal_workspace_persists_ended_state_after_a_host_restart() {
        let root = tempfile::tempdir().unwrap();
        let core = fixture(root.path());
        // A crashed host can leave a running checkpoint without a live PTY.
        let _: Value = state_call(
            &core,
            json!({"op":"create", "key":"terminal:w:pane", "record":{
                "workspaceId":"w", "terminalId":"pane", "generation":"before-crash",
                "title":"Shell", "cwd":root.path(), "status":"running", "rows":24, "cols":80,
                "createdAt":"now", "agentId":null, "exitCode":null, "historyTruncated":false
            }}),
        )
        .unwrap();
        core.terminal_state.lock().unwrap().take();
        assert_eq!(workspace(&core, "w").unwrap().terminals[0].status, "exited");
        let persisted: Vec<TerminalRecord> =
            state_call(&core, json!({"op":"list", "workspaceId":"w"})).unwrap();
        assert_eq!(persisted[0].status, "exited");
        let snapshot = snapshot(&core, "w", "pane").unwrap();
        assert_eq!(snapshot.record.generation, "before-crash");
        assert_eq!(snapshot.sequence, 1);
        assert!(core.runtimes.lock().unwrap().is_empty());
    }

    #[test]
    fn terminal_workspace_rejects_invalid_launches_before_spawning() {
        let root = tempfile::tempdir().unwrap();
        let core = fixture(root.path());
        assert!(create(
            &core,
            &CreateTerminalParams {
                terminal_id: "../bad".into(),
                ..params()
            }
        )
        .is_err());
        assert!(create(
            &core,
            &CreateTerminalParams {
                agent_id: Some("not-an-agent".into()),
                ..params()
            }
        )
        .is_err());
        assert!(create(
            &core,
            &CreateTerminalParams {
                cwd: Some("/bridge-missing-directory".into()),
                ..params()
            }
        )
        .is_err());
        assert!(core.runtimes.lock().unwrap().is_empty());
    }
}

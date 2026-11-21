//! Persistent terminal workspaces. Rust owns PTYs; the same xterm headless
//! engine used by Orca owns VT checkpoints and the ordered output journal.
use crate::{diagnostics, events::CoreEvent, BridgeCore, BridgeError, RuntimeSession};
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
use std::time::{Duration, Instant};

/// Hand a PTY writer to a thread of its own and return the queue that feeds
/// it. Callers enqueue and return at once: a program that stops reading its
/// input (a paused TUI, `cat` in canonical mode past MAX_CANON) blocks this
/// thread only, never a caller holding the shared runtimes map. The channel
/// keeps bytes in the order they were sent. The thread ends when the session
/// is dropped (the sender goes away) or the PTY refuses a write.
pub(crate) fn spawn_pty_writer(mut writer: Box<dyn Write + Send>) -> mpsc::Sender<Vec<u8>> {
    let (input, queue) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        for bytes in queue {
            if writer.write_all(&bytes).and_then(|()| writer.flush()).is_err() {
                break;
            }
        }
    });
    input
}

/// Read from a PTY, retrying a read a signal interrupted. `EINTR` is not the
/// end of the stream: treating it as one ended a live shell.
pub(crate) fn read_pty(reader: &mut dyn Read, buffer: &mut [u8]) -> std::io::Result<usize> {
    loop {
        match reader.read(buffer) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            other => return other,
        }
    }
}

/// Collect a PTY child's exit once its output has ended. The slave side
/// closes a moment before the process becomes reapable, so poll briefly
/// before deciding it is stuck; killing first threw away the exit code of
/// every shell that ended normally. A child that really is stuck is killed
/// and then waited on, so it never lingers as a zombie.
pub(crate) fn settle_exit(child: &mut (dyn portable_pty::Child + Send + Sync)) -> Option<u32> {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Some(status.exit_code());
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    child.wait().ok().map(|status| status.exit_code())
}

/// Reap a workspace terminal whose output has ended, leaving it registered
/// while it settles. `None` means a newer shell owns the key and this reader
/// has nothing left to do; `Some(None)` means the terminal was closed (or the
/// exit could not be read) and the record ends without a code.
fn settle_registered(core: &BridgeCore, key: &str, epoch: u64) -> Option<Option<u32>> {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        {
            let mut runtimes = core.runtimes.lock().unwrap();
            match runtimes.get_mut(key) {
                None => return Some(None),
                Some(runtime) if runtime.epoch != epoch => return None,
                Some(runtime) => {
                    if let Ok(Some(status)) = runtime.child.try_wait() {
                        return Some(Some(status.exit_code()));
                    }
                    if Instant::now() >= deadline {
                        let Some(mut runtime) = runtimes.remove(key) else { return Some(None) };
                        drop(runtimes);
                        let _ = runtime.child.kill();
                        return Some(runtime.child.wait().ok().map(|status| status.exit_code()));
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// End a PTY child and reap it. `kill` sends SIGHUP, waits briefly, then
/// SIGKILL; the wait collects the process so a closed terminal leaves no
/// zombie behind in a long-lived daemon.
pub(crate) fn end_child(child: &mut (dyn portable_pty::Child + Send + Sync)) {
    let _ = child.kill();
    let _ = child.wait();
}

/// The locale a PTY gets when Bridge itself has none. A Finder-launched app
/// inherits no `LANG`, and a shell without a UTF-8 locale makes TUIs fall back
/// to ASCII or measure wide characters wrongly. Terminal.app sets one for the
/// same reason. An existing locale always wins.
fn default_locale(lookup: impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<&'static str> {
    let set = |name: &str| lookup(name).is_some_and(|value| !value.is_empty());
    if set("LC_ALL") || set("LC_CTYPE") || set("LANG") {
        return None;
    }
    Some(if cfg!(target_os = "macos") { "en_US.UTF-8" } else { "C.UTF-8" })
}

/// The environment every Bridge PTY shares: a terminal type xterm.js
/// implements, a UTF-8 locale, and the hydrated PATH. Login shells rebuild
/// PATH from their profile anyway; an agent CLI started directly does not, and
/// with the GUI PATH its `#!/usr/bin/env node` line fails at once.
pub(crate) fn pty_environment(command: &mut CommandBuilder) {
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command.env("TERM_PROGRAM", "Bridge");
    if let Some(locale) = default_locale(|name| std::env::var_os(name)) {
        command.env("LANG", locale);
    }
    if let Some(path) = crate::binary::hydrated_command_path() {
        command.env("PATH", path);
    }
}

/// UTF-8 decoding across read boundaries. A PTY read can end inside a
/// multi-byte character; decoding each read on its own turned the halves into
/// replacement characters. The incomplete tail (at most three bytes) waits
/// for the next read.
#[derive(Default)]
pub(crate) struct Utf8Stream {
    pending: Vec<u8>,
}

impl Utf8Stream {
    pub(crate) fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut text = String::new();
        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(valid) => {
                    text.push_str(valid);
                    self.pending.clear();
                    return text;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    text.push_str(std::str::from_utf8(&self.pending[..valid]).unwrap_or_default());
                    match error.error_len() {
                        Some(invalid) => {
                            text.push('\u{FFFD}');
                            self.pending.drain(..valid + invalid);
                        }
                        None => {
                            self.pending.drain(..valid);
                            return text;
                        }
                    }
                }
            }
        }
    }
}

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
        #[cfg(target_os = "linux")]
        if let Some(path) = crate::binary::linux_resource_path(
            &exe, "sidecar/terminal-state/index.mjs",
        ) {
            candidates.push(path);
        }
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

/// Run `work` against the history sidecar, starting it on first use and
/// dropping it when it died so the next call starts a fresh one.
fn with_sidecar<T>(
    core: &BridgeCore,
    slot: &mut Option<StateSidecar>,
    work: impl FnOnce(&mut StateSidecar) -> Result<T, BridgeError>,
) -> Result<T, BridgeError> {
    if slot.is_none() {
        *slot = Some(StateSidecar::start(core)?);
    }
    let Some(sidecar) = slot.as_mut() else {
        return Err(BridgeError::Invalid("Terminal history runtime is unavailable".into()));
    };
    let result = work(sidecar);
    if sidecar.child.try_wait().ok().flatten().is_some() {
        slot.take();
    }
    result
}

fn state_call<T: DeserializeOwned>(core: &BridgeCore, request: Value) -> Result<T, BridgeError> {
    let mut slot = core.terminal_state.lock().unwrap();
    let result = with_sidecar(core, &mut slot, |sidecar| sidecar.call(request))?;
    serde_json::from_value(result)
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
    let _operation = crate::runtime::lock_operation(&operation);
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
    let _operation = crate::runtime::lock_operation(&operation);
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
    let _operation = crate::runtime::lock_operation(&operation);
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
    pty_environment(&mut command);
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
            input: spawn_pty_writer(writer),
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
            let count = match read_pty(&mut reader, &mut bytes) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let response: Value = match state_call(
                &core,
                json!({"op":"write", "key":runtime_id, "generation":owned.generation, "bytes":base64::engine::general_purpose::STANDARD.encode(&bytes[..count])}),
            ) {
                Ok(response) => response,
                Err(error) => {
                    diagnostics::record(&format!("terminal history failed: {error}"));
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
        // Reap and record the exit while the runtime is still registered.
        // Unregistering first left a window in which a snapshot saw a
        // running record with no live PTY and wrote it as exited with no
        // exit code, the recovery path meant for a crashed host.
        let Some(exit_code) = settle_registered(&core, &runtime_id, epoch) else { return };
        let _: Result<TerminalRecord, _> = state_call(
            &core,
            json!({"op":"update", "key":runtime_id, "changes":{"generation":owned.generation,"status":"exited","exitCode":exit_code}}),
        );
        {
            let mut runtimes = core.runtimes.lock().unwrap();
            if runtimes.get(&runtime_id).is_some_and(|runtime| runtime.epoch == epoch) {
                runtimes.remove(&runtime_id);
            }
        }
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
    let runtime_key = key(workspace, terminal);
    // The shared runtimes map is held for the ioctl alone. It used to be held
    // across two sidecar round trips, stalling every terminal's input and
    // every session stop behind a resize.
    let resize_pty = || -> Result<bool, BridgeError> {
        let runtimes = core.runtimes.lock().unwrap();
        let Some(runtime) = runtimes.get(&runtime_key) else {
            return Ok(false);
        };
        runtime
            .master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| BridgeError::Pty(e.to_string()))?;
        Ok(true)
    };
    if workspace == "provider-login" {
        return resize_pty().map(|_| ());
    }
    if !is_live(core, workspace, terminal) {
        return Ok(());
    }
    // The history lock spans the PTY resize and its journal record, so output
    // the program redraws after SIGWINCH is journalled after the resize.
    let mut slot = core.terminal_state.lock().unwrap();
    let resized = with_sidecar(core, &mut slot, |sidecar| {
        let snapshot = sidecar.call(json!({"op":"describe", "key":runtime_key}))?;
        let generation = snapshot["record"]["generation"].as_str().unwrap_or("").to_owned();
        if !resize_pty()? {
            return Ok(None);
        }
        let result = sidecar.call(json!({"op":"resize", "key":runtime_key, "generation":generation, "rows":rows, "cols":cols}))?;
        Ok(Some((generation, result["sequence"].as_u64().unwrap_or(0))))
    })?;
    drop(slot);
    if let Some((generation, sequence)) = resized {
        core.events.publish(CoreEvent::TerminalFrame(TerminalFrame {
            workspace_id: workspace.into(),
            terminal_id: terminal.into(),
            generation,
            sequence,
            data: String::new(),
            rows: Some(rows),
            cols: Some(cols),
            status: None,
        }));
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

    /// A runtime under `terminal:w:<id>` whose input lands in `writer`, backed
    /// by a real PTY child so the session is indistinguishable from a shell.
    fn fake_runtime(core: &Arc<BridgeCore>, id: &str, writer: Box<dyn Write + Send>) -> u32 {
        fake_runtime_running(core, id, writer, "sleep 30")
    }

    fn fake_runtime_running(core: &Arc<BridgeCore>, id: &str, writer: Box<dyn Write + Send>, script: &str) -> u32 {
        let pair = native_pty_system()
            .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
            .unwrap();
        let mut sleeper = CommandBuilder::new("/bin/sh");
        sleeper.args(["-c", script]);
        let child = pair.slave.spawn_command(sleeper).unwrap();
        let pid = child.process_id().unwrap();
        core.runtimes.lock().unwrap().insert(
            key("w", id),
            RuntimeSession { input: spawn_pty_writer(writer), master: pair.master, child, epoch: 0 },
        );
        pid
    }

    #[derive(Clone, Default)]
    struct Capture(Arc<std::sync::Mutex<Vec<u8>>>);
    impl Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl Capture {
        fn wait_for(&self, length: usize) -> Vec<u8> {
            let deadline = Instant::now() + Duration::from_secs(5);
            while self.0.lock().unwrap().len() < length {
                assert!(Instant::now() < deadline, "input never reached the writer");
                std::thread::sleep(Duration::from_millis(5));
            }
            self.0.lock().unwrap().clone()
        }
    }

    /// A writer whose program never reads: every write blocks until the test
    /// lets go.
    struct Stuck(mpsc::Receiver<()>);
    impl Write for Stuck {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            let _ = self.0.recv();
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_program_that_stops_reading_blocks_only_its_own_terminal() {
        let root = tempfile::tempdir().unwrap();
        let core = fixture(root.path());
        let (release, stuck) = mpsc::channel();
        let capture = Capture::default();
        fake_runtime(&core, "stuck", Box::new(Stuck(stuck)));
        fake_runtime(&core, "other", Box::new(capture.clone()));
        let started = Instant::now();
        crate::api::write_terminal(&core, "w", "stuck", &"x".repeat(64 * 1024)).unwrap();
        crate::api::write_terminal(&core, "w", "stuck", "more").unwrap();
        assert_eq!(crate::api::list_terminals(&core, "w"), vec!["other", "stuck"]);
        crate::api::write_terminal(&core, "w", "other", "still typing").unwrap();
        assert_eq!(capture.wait_for(12), b"still typing");
        assert!(started.elapsed() < Duration::from_secs(2), "a blocked PTY write held the runtimes map");
        drop(release);
        for id in ["stuck", "other"] {
            crate::api::close_terminal(&core, "w", id).unwrap();
        }
    }

    #[test]
    fn writes_arrive_in_order_and_binary_input_stays_bytes() {
        let root = tempfile::tempdir().unwrap();
        let core = fixture(root.path());
        let capture = Capture::default();
        fake_runtime(&core, "pane", Box::new(capture.clone()));
        let typed: String = (0..200).map(|n| char::from(b'a' + (n % 26) as u8)).collect();
        for char in typed.chars() {
            crate::api::write_terminal(&core, "w", "pane", &char.to_string()).unwrap();
        }
        // An X10 mouse report for column 223: the coordinate byte is 0xFF.
        crate::api::write_terminal_binary(&core, "w", "pane", "\u{1b}[M \u{ff}!").unwrap();
        crate::api::write_terminal(&core, "w", "pane", "\u{ff}").unwrap();
        let mut expected = typed.into_bytes();
        expected.extend_from_slice(&[0x1b, b'[', b'M', b' ', 0xff, b'!']);
        expected.extend_from_slice("\u{ff}".as_bytes());
        assert_eq!(capture.wait_for(expected.len()), expected);
        assert!(
            crate::api::write_terminal_binary(&core, "w", "pane", "\u{100}").is_err(),
            "a binary string holds bytes only"
        );
        crate::api::close_terminal(&core, "w", "pane").unwrap();
    }

    #[test]
    fn close_reaps_the_child_instead_of_leaving_a_zombie() {
        let root = tempfile::tempdir().unwrap();
        let core = fixture(root.path());
        // A program that ignores SIGHUP needs the SIGKILL that follows it, and
        // nothing reaped that kill.
        let pid = fake_runtime_running(&core, "pane", Box::new(Capture::default()), "trap '' HUP; sleep 30");
        std::thread::sleep(Duration::from_millis(100));
        crate::api::close_terminal(&core, "w", "pane").unwrap();
        // A zombie still answers signal 0; a reaped pid does not.
        let alive = unsafe { libc::kill(pid as libc::pid_t, 0) } == 0;
        assert!(!alive, "closed terminal {pid} was left unreaped");
    }

    #[test]
    fn a_shell_that_exits_keeps_its_exit_code() {
        let root = tempfile::tempdir().unwrap();
        let core = fixture(root.path());
        create(&core, &params()).unwrap();
        crate::api::write_terminal(&core, "w", "pane", "exit 3\n").unwrap();
        let ended = wait_for(&core, |s| s.record.status == "exited");
        assert_eq!(ended.record.exit_code, Some(3));
    }

    #[test]
    fn ptys_get_a_utf8_locale_unless_one_is_set_and_the_hydrated_path() {
        let none = |_: &str| None;
        assert!(default_locale(none).is_some_and(|locale| locale.ends_with("UTF-8")));
        let lang = |name: &str| (name == "LANG").then(|| "de_DE.UTF-8".into());
        assert_eq!(default_locale(lang), None, "an existing locale wins");
        let ctype = |name: &str| (name == "LC_CTYPE").then(|| "UTF-8".into());
        assert_eq!(default_locale(ctype), None);
        let empty = |name: &str| (name == "LANG").then(|| "".into());
        assert!(default_locale(empty).is_some(), "an empty LANG is no locale");
        let mut command = CommandBuilder::new("true");
        pty_environment(&mut command);
        assert_eq!(command.get_env("TERM"), Some(std::ffi::OsStr::new("xterm-256color")));
        assert_eq!(command.get_env("PATH").map(ToOwned::to_owned), crate::binary::hydrated_command_path());
    }

    #[test]
    fn utf8_split_across_reads_decodes_whole() {
        let mut stream = Utf8Stream::default();
        assert_eq!(stream.push(b"ab\xe2\x94"), "ab");
        assert_eq!(stream.push(b"\x80c"), "\u{2500}c");
        assert_eq!(stream.push(b"\xff!"), "\u{FFFD}!", "invalid bytes still surface");
        assert_eq!(stream.push(b""), "");
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

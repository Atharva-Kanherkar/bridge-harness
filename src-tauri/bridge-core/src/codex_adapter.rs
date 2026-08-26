use crate::{
    adapters::{AdapterRuntime, ResumeRequest, ShutdownReason, StartRequest},
    binary,
    context_inventory::{
        AdapterContextInventory, ContextInventoryScope, ContextLifecyclePhase, ContextSegmentClass,
        ContextSegmentObservation,
    },
    delegation::WriteMode,
    model::AuthState,
    BridgeError,
};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc, Mutex, MutexGuard, OnceLock,
    },
};
use uuid::Uuid;

pub struct CodexRuntime {
    pub writer: Arc<Mutex<ChildStdin>>,
    pub child: Child,
    pub thread_id: String,
    pub current_turn: Arc<Mutex<Option<String>>>,
    request_id: AtomicI64,
    sandbox_policy: Option<Value>,
    context_inventory: Mutex<Vec<AdapterContextInventory>>,
    stopped: bool,
    stderr_tail: crate::adapters::StderrTail,
}

pub struct StartedCodex {
    pub runtime: CodexRuntime,
    pub reader: BufReader<ChildStdout>,
    pub startup_messages: Vec<Value>,
}

pub fn start(request: StartRequest<'_>) -> Result<StartedCodex, BridgeError> {
    launch(request, None)
}

pub fn resume(request: ResumeRequest<'_>) -> Result<StartedCodex, BridgeError> {
    launch(
        StartRequest {
            cwd: request.cwd,
            model: request.model,
            effort: request.effort,
            instructions: request.instructions,
            write_mode: request.write_mode,
            read_only_sandbox: request.read_only_sandbox,
            briefing: request.briefing,
        },
        Some(request.provider_session_id),
    )
}

fn launch(
    request: StartRequest<'_>,
    resume_thread_id: Option<&str>,
) -> Result<StartedCodex, BridgeError> {
    let StartRequest {
        cwd,
        model,
        effort,
        instructions,
        write_mode,
        read_only_sandbox,
        briefing,
    } = request;
    // Codex cannot express one connector tool's exact identity, so a briefing
    // policy here would be decoration. Refused at the boundary with the reason,
    // never accepted-and-ignored.
    if briefing.is_some() {
        return Err(BridgeError::Invalid(
            crate::briefing_policy::adapter_may_brief("codex")
                .err()
                .map(|error| error.reason())
                .unwrap_or_else(|| "Codex cannot enforce briefing authority".into()),
        ));
    }
    let binary = resolve_runtime()
        .ok_or_else(|| BridgeError::Invalid("Codex binary is not installed".into()))?;
    let mut command = crate::worker_sandbox::command(&binary, read_only_sandbox)?;
    let sandbox_policy = read_only_sandbox.map(|sandbox| {
        json!({
            "type": "workspaceWrite",
            "writableRoots": [sandbox.output_dir().to_string_lossy()],
            "networkAccess": sandbox.network_allowed(),
        })
    });
    command
        .args(["app-server", "--listen", "stdio://"])
        .current_dir(
            read_only_sandbox
                .map(|sandbox| sandbox.output_dir())
                .unwrap_or_else(|| std::path::Path::new(cwd)),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Piped and tail-captured: a worker that dies before its typed result
        // reports the provider's own error, not a generic exit.
        .stderr(Stdio::piped());
    if let Some(sandbox) = read_only_sandbox {
        prepare_isolated_codex_home(sandbox)?;
        command
            .env("HOME", sandbox.output_dir())
            .env("TMPDIR", sandbox.output_dir())
            .env("BRIDGE_WORKER_OUTPUT_DIR", sandbox.output_dir());
    }
    crate::adapters::configure_process_group(&mut command);
    let mut child = command.spawn()?;
    let stderr_tail = crate::adapters::StderrTail::capture(&mut child);
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| BridgeError::Invalid("Codex app-server stdin unavailable".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| BridgeError::Invalid("Codex app-server stdout unavailable".into()))?;
    let writer = Arc::new(Mutex::new(stdin));
    let mut reader = BufReader::new(stdout);
    write_value(
        &writer,
        &json!({"method":"initialize","id":1,"params":{"clientInfo":{"name":"bridge","title":"Bridge","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":false,"requestAttestation":false}}}),
    )?;
    let (_, mut startup_messages) = wait_for_response(&mut reader, 1)?;
    write_value(&writer, &json!({"method":"initialized"}))?;
    let (method, params, lifecycle_phase) = if let Some(thread_id) = resume_thread_id {
        (
            "thread/resume",
            thread_resume_params(thread_id, cwd, model, instructions, write_mode),
            ContextLifecyclePhase::Resume,
        )
    } else {
        (
            "thread/start",
            thread_start_params(cwd, model, effort, instructions, write_mode),
            ContextLifecyclePhase::Start,
        )
    };
    write_value(&writer, &json!({"method":method,"id":2,"params":params}))?;
    let (response, mut later_messages) = wait_for_response(&mut reader, 2)?;
    startup_messages.append(&mut later_messages);
    let thread_id = response
        .pointer("/result/thread/id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            BridgeError::Invalid(format!(
                "Codex app-server returned no thread id: {response}"
            ))
        })?
        .to_owned();
    Ok(StartedCodex {
        runtime: CodexRuntime {
            writer,
            child,
            thread_id,
            current_turn: Arc::new(Mutex::new(None)),
            request_id: AtomicI64::new(10),
            sandbox_policy,
            context_inventory: Mutex::new(codex_context_inventory(lifecycle_phase)?),
            stopped: false,
            stderr_tail,
        },
        reader,
        startup_messages,
    })
}

fn prepare_isolated_codex_home(
    sandbox: &crate::worker_sandbox::ReadOnlySandbox,
) -> Result<(), BridgeError> {
    let Some(home) = std::env::var_os("HOME") else {
        return Ok(());
    };
    let source_root = std::path::PathBuf::from(home).join(".codex");
    let isolated_root = sandbox.output_dir().join(".codex");
    std::fs::create_dir_all(&isolated_root)?;
    for filename in ["auth.json", "config.toml"] {
        let source = source_root.join(filename);
        if !source.is_file() {
            continue;
        }
        let destination = isolated_root.join(filename);
        if destination.exists() {
            continue;
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&source, &destination)?;
        #[cfg(not(unix))]
        {
            let _ = (source, destination);
            return Err(BridgeError::Invalid(
                "Read-only Codex authentication projection is unsupported on this platform".into(),
            ));
        }
    }
    Ok(())
}

fn sandbox_settings(write_mode: Option<WriteMode>) -> (&'static str, &'static str) {
    match write_mode {
        None => ("never", "danger-full-access"),
        Some(WriteMode::ReadOnly) => ("on-request", "workspace-write"),
        Some(WriteMode::Shared | WriteMode::Isolated) => ("on-request", "workspace-write"),
        Some(WriteMode::Full) => ("never", "danger-full-access"),
    }
}

fn thread_start_params(
    cwd: &str,
    model: Option<&str>,
    effort: Option<&str>,
    instructions: Option<&str>,
    write_mode: Option<WriteMode>,
) -> Value {
    let (approval_policy, sandbox) = sandbox_settings(write_mode);
    let mut params = json!({"cwd":cwd,"approvalPolicy":approval_policy,"sandbox":sandbox,"ephemeral":false,"serviceName":"Bridge"});
    if let Some(model) = model.map(str::trim).filter(|value| !value.is_empty()) {
        params["model"] = json!(model);
    }
    if let Some(effort) = effort.map(str::trim).filter(|value| !value.is_empty()) {
        // Reasoning-effort override. Field names accepted by current Codex
        // app-server builds; unknown fields are ignored safely on older ones,
        // and the worker briefing also states the effort so behavior follows.
        params["effort"] = json!(effort);
        params["model_reasoning_effort"] = json!(effort);
    }
    if let Some(instructions) = instructions
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        // Accepted by current Codex app-server builds; unknown fields are ignored safely on older ones.
        params["developerInstructions"] = json!(instructions);
        params["instructions"] = json!(instructions);
    }
    params
}

fn thread_resume_params(
    thread_id: &str,
    cwd: &str,
    model: Option<&str>,
    instructions: Option<&str>,
    write_mode: Option<WriteMode>,
) -> Value {
    let (approval_policy, sandbox) = sandbox_settings(write_mode);
    let mut params = json!({
        "threadId": thread_id,
        "cwd": cwd,
        "approvalPolicy": approval_policy,
        "sandbox": sandbox,
    });
    if let Some(model) = model.map(str::trim).filter(|value| !value.is_empty()) {
        params["model"] = json!(model);
    }
    if let Some(instructions) = instructions
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        params["developerInstructions"] = json!(instructions);
    }
    params
}

pub fn supports_native_resume() -> bool {
    static SUPPORTS: OnceLock<bool> = OnceLock::new();
    *SUPPORTS.get_or_init(discover_native_resume)
}

fn discover_native_resume() -> bool {
    let Some(binary) = resolve_runtime() else {
        return false;
    };
    let output_dir = std::env::temp_dir().join(format!("bridge-codex-schema-{}", Uuid::new_v4()));
    let generated = Command::new(binary)
        .args([
            "app-server",
            "generate-json-schema",
            "--experimental",
            "--out",
        ])
        .arg(&output_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    let supported = generated
        && std::fs::read_to_string(output_dir.join("ClientRequest.json"))
            .is_ok_and(|schema| schema_supports_resume(&schema));
    let _ = std::fs::remove_dir_all(output_dir);
    supported
}

fn schema_supports_resume(schema: &str) -> bool {
    schema.contains("thread/resume") && schema.contains("ThreadResumeParams")
}

impl CodexRuntime {
    fn terminate(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        let _ = crate::adapters::terminate_process_group(self.child.id());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
    pub fn start_turn(
        &self,
        text: &str,
        application_context: Option<&str>,
    ) -> Result<(), BridgeError> {
        self.request(
            "turn/start",
            turn_start_params(
                &self.thread_id,
                text,
                application_context,
                self.sandbox_policy.as_ref(),
            ),
        )?;
        crate::context_inventory::record_runtime_inventory(
            &self.context_inventory,
            codex_context_inventory(ContextLifecyclePhase::PerTurn)?,
        );
        Ok(())
    }
    pub fn interrupt(&self) -> Result<(), BridgeError> {
        let turn_id = self
            .current_turn
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| BridgeError::Invalid("No active turn to stop".into()))?;
        self.request(
            "turn/interrupt",
            json!({"threadId":self.thread_id,"turnId":turn_id}),
        )
    }
    pub fn respond(&self, request_id: Value, decision: &str) -> Result<(), BridgeError> {
        write_value(
            &self.writer,
            &json!({"id":request_id,"result":{"decision":decision}}),
        )
    }
    fn request(&self, method: &str, params: Value) -> Result<(), BridgeError> {
        let id = self.request_id.fetch_add(1, Ordering::Relaxed);
        write_value(
            &self.writer,
            &json!({"method":method,"id":id,"params":params}),
        )
    }
}

fn turn_start_params(
    thread_id: &str,
    text: &str,
    application_context: Option<&str>,
    sandbox_policy: Option<&Value>,
) -> Value {
    let mut params =
        json!({"threadId":thread_id,"input":[{"type":"text","text":text,"text_elements":[]}]});
    if let Some(context) = application_context
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        params["additionalContext"] = json!({
            "bridge.credentials": {"kind": "application", "value": context}
        });
    }
    if let Some(sandbox_policy) = sandbox_policy {
        params["sandboxPolicy"] = sandbox_policy.clone();
    }
    params
}

impl AdapterRuntime for CodexRuntime {
    fn process_id(&self) -> u32 {
        self.child.id()
    }
    fn provider_session_id(&self) -> &str {
        &self.thread_id
    }
    fn current_turn(&self) -> Arc<Mutex<Option<String>>> {
        self.current_turn.clone()
    }
    fn context_inventory(&self) -> Vec<AdapterContextInventory> {
        self.context_inventory.lock().unwrap().clone()
    }
    fn send_turn(&self, text: &str) -> Result<(), BridgeError> {
        self.start_turn(text, None)
    }
    fn send_turn_with_context(
        &self,
        text: &str,
        application_context: &str,
    ) -> Result<(), BridgeError> {
        self.start_turn(text, Some(application_context))
    }
    fn interrupt(&self) -> Result<(), BridgeError> {
        CodexRuntime::interrupt(self)
    }
    fn respond(&self, request_id: Value, decision: &str) -> Result<(), BridgeError> {
        CodexRuntime::respond(self, request_id, decision)
    }
    fn read_usage(&self) -> Result<(), BridgeError> {
        // `account/rateLimits/read` is a read-only account query (no quota cost).
        // Its response lands on the event stream and is normalized to usage.updated.
        // The protocol requires a null params field.
        self.request("account/rateLimits/read", Value::Null)
    }
    fn failure_context(&mut self) -> Option<String> {
        crate::adapters::process_failure_context(&mut self.child, &self.stderr_tail)
    }
    fn stop(&mut self, _reason: ShutdownReason) {
        self.terminate();
    }
}

pub(crate) fn codex_context_inventory(
    lifecycle_phase: ContextLifecyclePhase,
) -> Result<Vec<AdapterContextInventory>, BridgeError> {
    let observations = || {
        vec![
            ContextSegmentObservation::unavailable(
                ContextSegmentClass::ProviderBaseInstructions,
                match lifecycle_phase {
                    ContextLifecyclePhase::Start => "Codex app-server does not expose the provider base instructions combined with thread/start",
                    ContextLifecyclePhase::Resume => "Codex app-server does not expose the provider base instructions retained or recomputed by thread/resume",
                    ContextLifecyclePhase::PerTurn => "Codex app-server does not expose the provider base instructions presented to turn/start",
                },
            ),
            ContextSegmentObservation::unavailable(
                ContextSegmentClass::ToolSchemas,
                "Codex app-server does not report provider-owned tool schemas presented to the model",
            ),
            ContextSegmentObservation::unavailable(
                ContextSegmentClass::McpDynamicTools,
                "Codex app-server does not report which MCP or dynamic tools are presented to this turn",
            ),
            ContextSegmentObservation::unavailable(
                ContextSegmentClass::SkillsPlugins,
                "Codex app-server does not report which skills or plugins contribute model context",
            ),
            ContextSegmentObservation::unavailable(
                ContextSegmentClass::AgentDefinitions,
                "Codex app-server does not report provider-owned agent definitions presented to the model",
            ),
        ]
    };
    let mut inventories = Vec::new();
    if lifecycle_phase != ContextLifecyclePhase::PerTurn {
        inventories.push(AdapterContextInventory::new(
            "codex",
            ContextInventoryScope::Catalog,
            lifecycle_phase,
            observations(),
        )?);
    }
    inventories.push(AdapterContextInventory::new(
        "codex",
        ContextInventoryScope::TurnPresented,
        lifecycle_phase,
        observations(),
    )?);
    Ok(inventories)
}

impl Drop for CodexRuntime {
    fn drop(&mut self) {
        self.terminate();
    }
}

/// Prefer a Bridge-managed payload, falling back to whatever the user already
/// has on PATH.
///
/// A user-managed `codex` keeps working exactly as before when no managed
/// payload is installed, and is never claimed or removed by Bridge.
pub fn resolve_runtime() -> Option<std::path::PathBuf> {
    crate::managed_runtime::managed_entrypoint("codex").or_else(|| binary::resolve("codex"))
}

pub fn binary_version() -> Option<String> {
    // Report the version of the copy that will actually launch, so a managed
    // payload is not described by whatever happens to be on PATH.
    binary::version_at(&resolve_runtime()?)
}

/// Whether `~/.codex/auth.json` parses with a non-empty token payload —
/// independent of whether the `codex` binary itself resolves.
pub fn auth_state() -> AuthState {
    auth_state_from_home(std::env::var_os("HOME").map(PathBuf::from))
}

fn auth_state_from_home(home: Option<PathBuf>) -> AuthState {
    let Some(home) = home else {
        return AuthState::Unknown;
    };
    // Metadata only: Bridge never opens or parses credential contents.
    let Ok(metadata) = std::fs::metadata(home.join(".codex/auth.json")) else {
        return AuthState::SignedOut;
    };
    if metadata.len() > 0 {
        AuthState::SignedIn
    } else {
        AuthState::SignedOut
    }
}

fn write_value(writer: &Arc<Mutex<ChildStdin>>, value: &Value) -> Result<(), BridgeError> {
    let mut writer = lock_writer(writer, "Codex")?;
    serde_json::to_writer(&mut *writer, value)
        .map_err(|e| BridgeError::Invalid(format!("Cannot encode adapter request: {e}")))?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}
fn lock_writer<'a, T>(
    writer: &'a Mutex<T>,
    provider: &str,
) -> Result<MutexGuard<'a, T>, BridgeError> {
    writer.lock().map_err(|_| {
        BridgeError::Adapter(format!(
            "{provider} stdin lock was poisoned; restart the session"
        ))
    })
}
fn wait_for_response(
    reader: &mut BufReader<ChildStdout>,
    id: i64,
) -> Result<(Value, Vec<Value>), BridgeError> {
    let mut skipped = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Err(BridgeError::Invalid(
                "Codex app-server closed during initialization".into(),
            ));
        }
        let value: Value = serde_json::from_str(line.trim())
            .map_err(|e| BridgeError::Invalid(format!("Invalid Codex app-server frame: {e}")))?;
        if value.get("id").and_then(Value::as_i64) == Some(id) {
            return Ok((value, skipped));
        }
        skipped.push(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_inventory::{ContextInventoryScope, ContextObservationProvenance};
    #[test]
    fn poisoned_writer_is_a_typed_adapter_error() {
        let writer = Mutex::new(());
        let _ = std::panic::catch_unwind(|| {
            let _guard = writer.lock().unwrap();
            panic!("provider thread failed");
        });
        assert!(matches!(
            lock_writer(&writer, "Codex"),
            Err(BridgeError::Adapter(_))
        ));
    }

    #[test]
    fn codex_context_inventory_covers_start_resume_and_per_turn() {
        let start = thread_start_params("/tmp/work", None, None, Some("bridge"), None);
        assert_eq!(start["instructions"], "bridge");
        assert_eq!(start["developerInstructions"], "bridge");
        let resume = thread_resume_params("thread", "/tmp/work", None, Some("bridge"), None);
        assert!(resume.get("instructions").is_none());
        assert_eq!(resume["developerInstructions"], "bridge");

        for phase in [
            ContextLifecyclePhase::Start,
            ContextLifecyclePhase::Resume,
            ContextLifecyclePhase::PerTurn,
        ] {
            let inventories = codex_context_inventory(phase).unwrap();
            assert!(inventories
                .iter()
                .any(|item| item.scope == ContextInventoryScope::TurnPresented));
            assert!(inventories.iter().flat_map(|item| &item.observations).all(
                |observation| matches!(
                    observation.provenance,
                    ContextObservationProvenance::Unavailable { ref reason }
                        if !reason.is_empty()
                )
            ));
        }
    }

    #[test]
    fn worker_sandbox_and_approval_follow_write_mode() {
        for mode in [
            WriteMode::ReadOnly,
            WriteMode::Shared,
            WriteMode::Isolated,
            WriteMode::Full,
        ] {
            let params = thread_start_params("/tmp/work", None, None, None, Some(mode));
            if mode == WriteMode::Full {
                assert_eq!(params["sandbox"], "danger-full-access");
                assert_eq!(params["approvalPolicy"], "never");
            } else {
                assert_eq!(params["sandbox"], "workspace-write");
                assert_eq!(params["approvalPolicy"], "on-request");
            }
        }
        let orchestrator = thread_start_params("/tmp/work", None, None, None, None);
        assert_eq!(orchestrator["sandbox"], "danger-full-access");
        assert_eq!(orchestrator["approvalPolicy"], "never");
    }

    #[test]
    fn thread_params_preserve_runtime_configuration() {
        let params = thread_start_params(
            "/tmp/work",
            Some("runtime-model"),
            Some("high"),
            Some("worker rules"),
            Some(WriteMode::ReadOnly),
        );
        assert_eq!(params["model"], "runtime-model");
        assert_eq!(params["effort"], "high");
        assert_eq!(params["developerInstructions"], "worker rules");
    }

    #[test]
    fn codex_receives_the_compiled_stable_prefix_before_variable_context() {
        let compile = |evidence: &str| {
            crate::prompt_compiler::PromptCompiler::new("worker:verification")
                .stable_section("contract", "stable-provider-contract")
                .variable_section("evidence", evidence)
                .compile()
                .unwrap()
        };
        let first = compile("variable-task-evidence-one");
        let second = compile("variable-task-evidence-two");
        assert_eq!(first.metadata.prefix_hash, second.metadata.prefix_hash);
        assert_eq!(first.stable_prefix, second.stable_prefix);
        assert_ne!(first.variable_suffix, second.variable_suffix);
        for prompt in [first, second] {
            let params = thread_start_params(
                "/tmp/work",
                Some("runtime-model"),
                None,
                Some(prompt.instructions()),
                Some(WriteMode::ReadOnly),
            );
            let instructions = params["developerInstructions"].as_str().unwrap();
            assert_eq!(params["instructions"], params["developerInstructions"]);
            assert!(instructions.starts_with("<bridge-stable-prompt"));
            assert!(
                instructions.find("stable-provider-contract").unwrap()
                    < instructions.find("variable-task-evidence").unwrap()
            );
        }
    }

    #[test]
    fn native_resume_capability_is_discovered_from_protocol_schema() {
        assert!(schema_supports_resume(
            r#"{"method":"thread/resume","params":{"$ref":"ThreadResumeParams"}}"#
        ));
        assert!(!schema_supports_resume(
            r#"{"method":"thread/start","params":{"$ref":"ThreadStartParams"}}"#
        ));
    }

    #[test]
    fn resume_request_uses_stored_thread_and_current_enforcement() {
        let params = thread_resume_params(
            "thread-existing",
            "/tmp/work",
            Some("runtime-model"),
            Some("restored rules"),
            Some(WriteMode::ReadOnly),
        );
        assert_eq!(params["threadId"], "thread-existing");
        assert_eq!(params["sandbox"], "workspace-write");
        assert_eq!(params["approvalPolicy"], "on-request");
        assert_eq!(params["developerInstructions"], "restored rules");
        assert!(params.get("ephemeral").is_none());
    }
    #[test]
    fn turn_request_is_structured_json_not_terminal_text() {
        let value = json!({"method":"turn/start","id":10,"params":{"threadId":"t","input":[{"type":"text","text":"hello","text_elements":[]}]}});
        assert_eq!(value["method"], "turn/start");
        assert!(value.to_string().contains("text_elements"));
        assert!(!value.to_string().contains("\\u001b"));
    }

    #[test]
    fn turn_request_attaches_bridge_context_without_changing_user_text() {
        let params = turn_start_params(
            "thread-existing",
            "verify [secret:sec_reference]",
            Some("trusted broker capability"),
            None,
        );
        assert_eq!(params["input"][0]["text"], "verify [secret:sec_reference]");
        assert_eq!(
            params["additionalContext"]["bridge.credentials"]["kind"],
            "application"
        );
        assert_eq!(
            params["additionalContext"]["bridge.credentials"]["value"],
            "trusted broker capability"
        );
    }

    #[test]
    fn read_only_turn_adds_only_the_assigned_output_root() {
        let policy = json!({
            "type": "workspaceWrite",
            "writableRoots": ["/tmp/bridge-output"],
            "networkAccess": false,
        });
        let params = turn_start_params("thread", "verify", None, Some(&policy));
        assert_eq!(params["sandboxPolicy"], policy);
        assert_eq!(
            params["sandboxPolicy"]["writableRoots"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    #[ignore = "requires an installed, authenticated Codex binary"]
    fn live_app_server_emits_a_structured_turn() {
        use std::{sync::mpsc, thread, time::Duration};
        let cwd = std::env::current_dir().unwrap();
        let started = start(StartRequest {
            cwd: cwd.to_str().unwrap(),
            model: None,
            effort: None,
            instructions: None,
            write_mode: None,
            read_only_sandbox: None,
            briefing: None,
        })
        .unwrap();
        let mut runtime = started.runtime;
        let mut reader = started.reader;
        runtime
            .start_turn("Reply exactly BRIDGE_SMOKE_OK. Do not use tools.", None)
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            if let Ok(value) = serde_json::from_str::<Value>(line.trim()) {
                let completed =
                    value.get("method").and_then(Value::as_str) == Some("turn/completed");
                let _ = sender.send(value);
                if completed {
                    break;
                }
            }
        });
        let mut methods = Vec::new();
        loop {
            let value = receiver
                .recv_timeout(Duration::from_secs(90))
                .expect("Codex turn timed out");
            if let Some(method) = value.get("method").and_then(Value::as_str) {
                methods.push(method.to_owned());
            }
            if value.get("method").and_then(Value::as_str) == Some("turn/completed") {
                break;
            }
        }
        runtime.stop(ShutdownReason::Completed);
        assert!(methods.iter().any(|method| method == "turn/started"));
        assert!(methods
            .iter()
            .any(|method| method == "item/agentMessage/delta" || method == "item/completed"));
        assert!(methods.iter().any(|method| method == "turn/completed"));
    }

    #[test]
    #[ignore = "requires an installed, authenticated Codex binary and persists a provider thread"]
    fn live_codex_thread_survives_process_restart() {
        fn run_turn(started: &mut StartedCodex, prompt: &str) -> String {
            started.runtime.start_turn(prompt, None).unwrap();
            let mut transcript = String::new();
            loop {
                let mut line = String::new();
                assert_ne!(started.reader.read_line(&mut line).unwrap(), 0);
                transcript.push_str(&line);
                let frame: Value = serde_json::from_str(line.trim()).unwrap();
                if frame.get("method").and_then(Value::as_str) == Some("turn/completed") {
                    return transcript;
                }
            }
        }

        let cwd = std::env::current_dir().unwrap();
        let cwd = cwd.to_str().unwrap();
        let mut started = start(StartRequest {
            cwd,
            model: None,
            effort: None,
            instructions: None,
            write_mode: None,
            read_only_sandbox: None,
            briefing: None,
        })
        .unwrap();
        run_turn(&mut started, "Remember this exact token for the next turn: BRIDGE_CODEX_RESUME_8F31. Reply only SAVED.");
        let thread_id = started.runtime.thread_id.clone();
        started.runtime.stop(ShutdownReason::AppShutdown);

        let mut resumed = resume(ResumeRequest {
            cwd,
            model: None,
            effort: None,
            instructions: None,
            write_mode: None,
            read_only_sandbox: None,
            briefing: None,
            provider_session_id: &thread_id,
        })
        .unwrap();
        let transcript = run_turn(
            &mut resumed,
            "What exact token did I ask you to remember? Reply with only the token.",
        );
        resumed.runtime.stop(ShutdownReason::Completed);
        assert!(transcript.contains("BRIDGE_CODEX_RESUME_8F31"));
    }

    #[test]
    fn auth_probe_reports_signed_in_for_any_nonempty_store_without_reading_contents() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".codex")).unwrap();
        std::fs::write(home.path().join(".codex/auth.json"), "{not valid json").unwrap();
        assert_eq!(
            auth_state_from_home(Some(home.path().to_path_buf())),
            AuthState::SignedIn
        );
    }

    #[test]
    fn auth_probe_reports_signed_out_when_cli_present_but_store_absent() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(
            auth_state_from_home(Some(home.path().to_path_buf())),
            AuthState::SignedOut
        );
    }

    #[test]
    fn auth_probe_reports_signed_out_when_store_is_empty() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".codex")).unwrap();
        std::fs::write(home.path().join(".codex/auth.json"), "").unwrap();
        assert_eq!(
            auth_state_from_home(Some(home.path().to_path_buf())),
            AuthState::SignedOut
        );
    }

    #[test]
    fn auth_probe_reports_unknown_when_home_is_missing() {
        assert_eq!(auth_state_from_home(None), AuthState::Unknown);
    }
}

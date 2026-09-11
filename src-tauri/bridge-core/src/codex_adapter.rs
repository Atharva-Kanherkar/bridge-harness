use crate::{
    adapters::{AdapterRuntime, ResumeRequest, ShutdownReason, StartRequest, TurnContext},
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

const MINIMUM_VERSION: (u64, u64, u64) = (0, 153, 4);

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

struct SpawnedChildGuard {
    child: Option<Child>,
}

impl SpawnedChildGuard {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("spawn guard owns child")
    }

    fn disarm(mut self) -> Child {
        self.child.take().expect("spawn guard owns child")
    }
}

impl Drop for SpawnedChildGuard {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = crate::adapters::terminate_process_group(child.id());
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub fn start(request: StartRequest<'_>) -> Result<StartedCodex, BridgeError> {
    launch(request, None, false)
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
            on_progress: request.on_progress,
        },
        Some(request.provider_session_id),
        request.fork,
    )
}

pub fn discover_models() -> Result<Vec<crate::adapters::DiscoveredModel>, BridgeError> {
    let binary = resolve_runtime().ok_or_else(|| BridgeError::Invalid("Codex binary is not installed".into()))?;
    ensure_supported_version(&binary)?;
    let mut command = crate::adapters::supervised_command(&binary, ["app-server", "--listen", "stdio://"]);
    binary::hydrate_command_path(&mut command);
    crate::adapters::configure_process_group(&mut command);
    let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    let stdin = child.stdin.take().ok_or_else(|| BridgeError::Adapter("Codex catalogue stdin unavailable".into()))?;
    let stdout = child.stdout.take().ok_or_else(|| BridgeError::Adapter("Codex catalogue stdout unavailable".into()))?;
    let writer = Arc::new(Mutex::new(stdin));
    let mut reader = BufReader::new(stdout);
    let result = (|| {
        write_value(&writer, &json!({"method":"initialize","id":1,"params":{"clientInfo":{"name":"bridge","title":"Bridge","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":false}}}))?;
        wait_for_response(&mut reader, 1)?;
        write_value(&writer, &json!({"method":"initialized"}))?;
        // Empty params: the server excludes hidden models by default.
        write_value(&writer, &json!({"method":"model/list","id":2,"params":{}}))?;
        let (response, _) = wait_for_response(&mut reader, 2)?;
        let rows = response.pointer("/result/data").or_else(|| response.pointer("/result/models")).and_then(Value::as_array)
            .ok_or_else(|| BridgeError::Adapter("Codex returned no model catalogue".into()))?;
        let models = rows.iter().filter_map(|row| {
            // Defensive: skip any hidden row even if the server sent one.
            if row.get("hidden").and_then(Value::as_bool).unwrap_or(false) {
                return None;
            }
            let id = row.get("id").or_else(|| row.get("model")).and_then(Value::as_str)?.trim();
            let label = row.get("displayName").or_else(|| row.get("name")).and_then(Value::as_str).unwrap_or(id).trim();
            let is_default = row.get("isDefault").and_then(Value::as_bool).unwrap_or(false);
            // Each supported effort is an object carrying its `reasoningEffort`
            // string (low/medium/high/xhigh/max/ultra); keep only those names.
            let supported_effort_levels = row.get("supportedReasoningEfforts")
                .and_then(Value::as_array)
                .map(|efforts| efforts.iter().filter_map(|effort| {
                    effort.get("reasoningEffort").and_then(Value::as_str)
                        .or_else(|| effort.as_str())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned)
                }).collect::<Vec<_>>())
                .unwrap_or_default();
            (!id.is_empty() && !label.is_empty()).then(|| crate::adapters::DiscoveredModel {
                id: id.to_owned(),
                label: label.to_owned(),
                is_default,
                supported_effort_levels,
            })
        }).collect::<Vec<_>>();
        if models.is_empty() { Err(BridgeError::Adapter("Codex returned an empty model catalogue".into())) } else { Ok(models) }
    })();
    let _ = crate::adapters::terminate_process_group(child.id());
    let _ = child.kill();
    let _ = child.wait();
    result
}

fn launch(
    request: StartRequest<'_>,
    resume_thread_id: Option<&str>,
    fork: bool,
) -> Result<StartedCodex, BridgeError> {
    let StartRequest {
        cwd,
        model,
        effort,
        instructions,
        write_mode,
        read_only_sandbox,
        briefing,
        on_progress,
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
    ensure_supported_version(&binary)?;
    let mut command = crate::worker_sandbox::command(&binary, read_only_sandbox)?;
    binary::hydrate_command_path(&mut command);
    let sandbox_policy = read_only_sandbox.map(|sandbox| {
        json!({
            "type": "workspaceWrite",
            "writableRoots": [sandbox.output_dir().to_string_lossy()],
            "networkAccess": sandbox.network_allowed(),
        })
    });
    command
        .args(["app-server", "--listen", "stdio://"])
        .current_dir(std::path::Path::new(cwd))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Piped and tail-captured: a worker that dies before its typed result
        // reports the provider's own error, not a generic exit.
        .stderr(Stdio::piped());
    if let Some(sandbox) = read_only_sandbox {
        let codex_home = prepare_isolated_codex_home(sandbox)?;
        command
            .env("CODEX_HOME", codex_home)
            .env("HOME", sandbox.output_dir())
            .env("TMPDIR", sandbox.output_dir())
            .env("BRIDGE_WORKER_OUTPUT_DIR", sandbox.output_dir());
        // The seatbelt cannot use the host keychain; a networked worker (e.g. a
        // PR review) needs the host token or every `gh` call returns 401.
        if sandbox.network_allowed() {
            if let Some(token) = crate::worker_sandbox::github_cli_token() {
                command.env("GH_TOKEN", token);
            }
        }
    }
    // Build output belongs to the repository, not to this checkout. Skipped for
    // a read-only worker: its seatbelt permits writes only under its own output
    // directory, and widening that to reach a shared cache would trade away part
    // of the read-only guarantee for the speed of a worker that is not meant to
    // be building.
    if read_only_sandbox.is_none() {
        crate::build_cache::apply(&mut command, std::path::Path::new(cwd));
    }
    crate::adapters::configure_process_group(&mut command);
    if let Some(on_progress) = on_progress {
        on_progress(crate::adapters::StartupPhase::Spawning);
    }
    let spawned_at = std::time::Instant::now();
    let mut child = SpawnedChildGuard::new(command.spawn()?);
    let stderr_tail = crate::adapters::StderrTail::capture(child.child_mut());
    let stdin = child
        .child_mut()
        .stdin
        .take()
        .ok_or_else(|| BridgeError::Invalid("Codex app-server stdin unavailable".into()))?;
    let stdout = child
        .child_mut()
        .stdout
        .take()
        .ok_or_else(|| BridgeError::Invalid("Codex app-server stdout unavailable".into()))?;
    let writer = Arc::new(Mutex::new(stdin));
    let mut reader = BufReader::new(stdout);
    write_value(
        &writer,
        &json!({"method":"initialize","id":1,"params":{"clientInfo":{"name":"bridge","title":"Bridge","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":false,"requestAttestation":false}}}),
    )?;
    if let Some(on_progress) = on_progress {
        on_progress(crate::adapters::StartupPhase::Handshake);
    }
    let (_, mut startup_messages) = wait_for_response(&mut reader, 1)?;
    crate::process_ledger::log_spawn_to_ready("codex", "initialize_response", spawned_at);
    write_value(&writer, &json!({"method":"initialized"}))?;
    let (method, params, lifecycle_phase) = if let Some(thread_id) = resume_thread_id.filter(|_| fork) {
        (
            "thread/fork",
            thread_fork_params(thread_id, cwd, model, instructions, write_mode),
            ContextLifecyclePhase::Resume,
        )
    } else if let Some(thread_id) = resume_thread_id {
        (
            "thread/resume",
            thread_resume_params(thread_id, cwd, model, effort, instructions, write_mode),
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
    if let Some(on_progress) = on_progress {
        on_progress(crate::adapters::StartupPhase::SessionOpen);
    }
    Ok(StartedCodex {
        runtime: CodexRuntime {
            writer,
            child: child.disarm(),
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
) -> Result<PathBuf, BridgeError> {
    let isolated_root = sandbox.output_dir().join(".codex");
    std::fs::create_dir_all(&isolated_root)?;
    let Some(home) = std::env::var_os("HOME") else {
        return Ok(isolated_root);
    };
    crate::capability_projection::project_read_only_capabilities(
        crate::capability_projection::CapabilityHarness::Codex,
        &PathBuf::from(home),
        sandbox.output_dir(),
    )?;
    Ok(isolated_root)
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
        // The generated schema for codex-cli 0.153.4 has no top-level `effort`
        // on either ThreadStartParams or ThreadResumeParams, but both accept
        // a permissive `config` map — and `model_reasoning_effort` is the
        // Codex config key for it — so carry it there too. Start and resume
        // agree on this shape; the top-level fields stay for any build that
        // did read them.
        params["effort"] = json!(effort);
        params["model_reasoning_effort"] = json!(effort);
        params["config"] = json!({ "model_reasoning_effort": effort });
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
    effort: Option<&str>,
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
    if let Some(effort) = effort.map(str::trim).filter(|value| !value.is_empty()) {
        // Resume previously sent no effort at all, so a switch that changed
        // model *and* effort resumed at the thread's previous effort. The
        // app-server schema has no top-level `effort` on ThreadResumeParams,
        // but it accepts a permissive `config` map, and
        // `model_reasoning_effort` is the Codex config key for it.
        params["config"] = json!({ "model_reasoning_effort": effort });
    }
    if let Some(instructions) = instructions
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        params["developerInstructions"] = json!(instructions);
    }
    params
}

/// Fork params mirror resume's, with one different verb: `thread/fork` loads
/// the source thread from disk and continues into a NEW thread, so the aside
/// reads the source's full history while every write lands on the fork. The
/// aside's own compiled instructions ride along as developer instructions.
fn thread_fork_params(
    thread_id: &str,
    cwd: &str,
    model: Option<&str>,
    instructions: Option<&str>,
    write_mode: Option<WriteMode>,
) -> Value {
    let mut params = thread_resume_params(thread_id, cwd, model, None, instructions, write_mode);
    params["threadSource"] = json!("bridge_side_chat");
    params
}

pub fn supports_native_resume() -> bool {
    static SUPPORTS: OnceLock<bool> = OnceLock::new();
    *SUPPORTS.get_or_init(|| schema_declares(schema_supports_resume))
}

/// Whether this Codex build exposes `thread/fork` — the native side-chat verb:
/// fork the source thread into a new one, read its full history, and never
/// write to the source. Discovered from the app-server schema the same way
/// native resume is, and cached for the process lifetime.
pub fn supports_native_fork() -> bool {
    static SUPPORTS: OnceLock<bool> = OnceLock::new();
    *SUPPORTS.get_or_init(|| schema_declares(schema_supports_fork))
}

/// Whether the installed Codex's app-server schema declares a capability.
///
/// One probe, shared: generating the schema costs a process launch, and three
/// near-copies of this walk is how the next capability ends up reading a
/// different file than the others. Each caller supplies only the predicate,
/// and caches its own answer for the process lifetime.
fn schema_declares(predicate: fn(&str) -> bool) -> bool {
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
            .is_ok_and(|schema| predicate(&schema));
    let _ = std::fs::remove_dir_all(output_dir);
    supported
}

/// Whether this Codex build exposes `thread/compact/start`, its own
/// compaction verb. Under the ownership split in
/// `docs/compaction-and-resume.md` this is what `/compact` reaches on a Codex
/// chat; the params carry a thread id and nothing else, so a focus cannot be
/// forwarded.
pub fn supports_native_compaction() -> bool {
    static SUPPORTS: OnceLock<bool> = OnceLock::new();
    *SUPPORTS.get_or_init(|| schema_declares(schema_supports_compact))
}

fn schema_supports_compact(schema: &str) -> bool {
    schema.contains("thread/compact/start") && schema.contains("ThreadCompactStartParams")
}

fn schema_supports_fork(schema: &str) -> bool {
    schema.contains("thread/fork") && schema.contains("ThreadForkParams")
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
    pub fn start_turn(&self, text: &str, context: TurnContext<'_>) -> Result<(), BridgeError> {
        self.start_turn_with_images(text, context, &[])
    }
    fn start_turn_with_images(&self, text: &str, context: TurnContext<'_>, images: &[bridge_protocol::messages::TurnImage]) -> Result<(), BridgeError> {
        let mut params = turn_start_params(&self.thread_id, text, context, self.sandbox_policy.as_ref());
        append_images(&mut params, images);
        self.request("turn/start", params)?;
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

/// Inline image bytes keep clipboard content independent of local file paths.
fn append_images(params: &mut Value, images: &[bridge_protocol::messages::TurnImage]) {
    params["input"].as_array_mut().expect("turn input array").extend(images.iter().map(|image| {
        json!({"type": "image", "url": format!("data:{};base64,{}", image.media_type, image.base64_data)})
    }));
}

/// The whole of `thread/compact/start`: Codex compacts the thread it is given
/// and takes no focus, no target size, and no summary instruction.
fn compact_start_params(thread_id: &str) -> Value {
    json!({"threadId": thread_id})
}

fn turn_start_params(
    thread_id: &str,
    text: &str,
    context: TurnContext<'_>,
    sandbox_policy: Option<&Value>,
) -> Value {
    // The session frame is a leading `input` item, not `additionalContext`.
    //
    // `TurnStartParams` documents eleven fields as applying "for this turn and
    // subsequent turns"; `additionalContext` is deliberately not one of them —
    // it is "context fragments" scoped to the turn that carries them. A
    // standing contract delivered there once would be gone by the next turn,
    // and the delivery ledger would still believe the thread held it. `input`
    // items are the turn's user message, so they persist in the thread exactly
    // like Claude's content blocks and OpenCode's parts, which is what
    // deliver-once needs.
    let mut input = Vec::new();
    if let Some(frame) = context.session.map(str::trim).filter(|f| !f.is_empty()) {
        input.push(json!({"type":"text","text":frame,"text_elements":[]}));
    }
    input.push(json!({"type":"text","text":text,"text_elements":[]}));
    let mut params = json!({"threadId":thread_id,"input":input});
    // Per-turn credential context keeps its existing home: it is re-sent on
    // every turn whose text carries a registered marker, so a turn-scoped
    // fragment is exactly right for it.
    if let Some(credentials) = context
        .credentials
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        params["additionalContext"] = json!({
            "bridge.credentials": {"kind": "application", "value": credentials}
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
        self.start_turn(text, TurnContext::default())
    }
    fn send_turn_with_context(
        &self,
        text: &str,
        context: TurnContext<'_>,
    ) -> Result<(), BridgeError> {
        self.start_turn(text, context)
    }
    fn supports_images(&self) -> bool { true }
    fn send_turn_with_images(&self, text: &str, context: TurnContext<'_>, images: &[bridge_protocol::messages::TurnImage]) -> Result<(), BridgeError> {
        self.start_turn_with_images(text, context, images)
    }
    fn interrupt(&self) -> Result<(), BridgeError> {
        CodexRuntime::interrupt(self)
    }
    /// `thread/compact/start` takes a thread id and nothing else, so Codex
    /// compacts the whole conversation and a focus cannot ride along.
    fn native_compaction(&self) -> crate::adapters::NativeCompaction {
        if supports_native_compaction() {
            crate::adapters::NativeCompaction::WholeConversation
        } else {
            crate::adapters::NativeCompaction::Unsupported
        }
    }
    fn compact_native(&self, _focus: Option<&str>) -> Result<(), BridgeError> {
        self.request("thread/compact/start", compact_start_params(&self.thread_id))
    }
    fn respond(&self, request_id: Value, decision: &str) -> Result<(), BridgeError> {
        CodexRuntime::respond(self, request_id, decision)
    }
    fn answer_question(&self, request_id: Value, result: Value) -> Result<(), BridgeError> {
        write_value(&self.writer, &json!({"id":request_id,"result":result}))
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

pub fn is_supported_version(version: &str) -> bool {
    version
        .split_whitespace()
        .find_map(|token| {
            let token = token.strip_prefix('v').unwrap_or(token);
            let parts = token.split('.').collect::<Vec<_>>();
            if parts.len() != 3
                || parts.iter().any(|part| {
                    part.is_empty()
                        || !part
                            .chars()
                            .all(|character| character.is_ascii_digit())
                })
            {
                return None;
            }
            Some((
                parts[0].parse::<u64>().ok()?,
                parts[1].parse::<u64>().ok()?,
                parts[2].parse::<u64>().ok()?,
            ))
        })
        .is_some_and(|version| version >= MINIMUM_VERSION)
}

fn ensure_supported_version(executable: &std::path::Path) -> Result<(), BridgeError> {
    let version = binary::version_at(executable).ok_or_else(|| {
        BridgeError::Invalid(format!(
            "Cannot read Codex version from {}",
            executable.display()
        ))
    })?;
    if is_supported_version(&version) {
        return Ok(());
    }
    Err(BridgeError::Invalid(format!(
        "Codex {version} is incompatible with Bridge. Upgrade to Codex 0.153.4 or newer."
    )))
}

/// Whether `~/.codex/auth.json` parses with a non-empty token payload —
/// independent of whether the `codex` binary itself resolves.
pub fn auth_state() -> AuthState {
    auth_state_from_environment(
        std::env::var_os("HOME").map(PathBuf::from),
        &crate::capability_projection::CapabilityEnvironment::from_process(),
    )
}

#[cfg(test)]
fn auth_state_from_home(home: Option<PathBuf>) -> AuthState {
    auth_state_from_environment(
        home,
        &crate::capability_projection::CapabilityEnvironment::default(),
    )
}

fn auth_state_from_environment(
    home: Option<PathBuf>,
    environment: &crate::capability_projection::CapabilityEnvironment,
) -> AuthState {
    let Some(home) = home else {
        return AuthState::Unknown;
    };
    let config = crate::capability_projection::user_config_root(
        crate::capability_projection::CapabilityHarness::Codex,
        &home,
        environment,
    );
    // Metadata only: Bridge never opens or parses credential contents.
    let Ok(metadata) = std::fs::metadata(config.join("auth.json")) else {
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
    #[test]
    fn image_attachments_keep_native_shapes_and_order() {
        let images = vec![
            bridge_protocol::messages::TurnImage { media_type: "image/png".into(), base64_data: "cG5n".into() },
            bridge_protocol::messages::TurnImage { media_type: "image/jpeg".into(), base64_data: "anBlZw==".into() },
        ];
        let mut payload = turn_start_params("thread", "describe", TurnContext::default(), None);
        append_images(&mut payload, &images);
        assert_eq!(payload["input"][0]["text"], "describe");
        assert_eq!(payload["input"][1], json!({"type":"image", "url":"data:image/png;base64,cG5n"}));
        assert_eq!(payload["input"][2], json!({"type":"image", "url":"data:image/jpeg;base64,anBlZw=="}));
    }

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
        let resume = thread_resume_params("thread", "/tmp/work", None, None, Some("bridge"), None);
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
    fn codex_version_gate_is_strict_and_matches_the_certified_runtime() {
        assert!(!is_supported_version("codex-cli 0.153.3"));
        assert!(is_supported_version("codex-cli 0.153.4"));
        assert!(is_supported_version("codex-cli 0.200.0"));
        assert!(is_supported_version("codex-cli 1.0.0"));
        assert!(!is_supported_version("build 9"));
        assert!(!is_supported_version("codex-cli 0.153"));
        assert!(!is_supported_version("codex-cli 0.153.4-beta.1"));
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
        assert_eq!(params["model_reasoning_effort"], "high");
        assert_eq!(params["config"]["model_reasoning_effort"], "high");
        assert_eq!(params["developerInstructions"], "worker rules");
    }

    #[test]
    fn thread_resume_carries_effort_as_config_and_omits_it_when_absent() {
        // A switch that changes model *and* effort resumes the stored thread,
        // so the resume must carry the effort — via the permissive `config`
        // map, the only place the 0.153.4 schema accepts it.
        let with_effort = thread_resume_params(
            "thread-existing",
            "/tmp/work",
            Some("runtime-model"),
            Some("high"),
            None,
            None,
        );
        assert_eq!(with_effort["config"]["model_reasoning_effort"], "high");

        let without_effort =
            thread_resume_params("thread-existing", "/tmp/work", None, None, None, None);
        assert!(without_effort.get("config").is_none());

        // Start and resume agree; the top-level fields stay for any build that
        // did read them.
        let start = thread_start_params("/tmp/work", None, Some("high"), None, None);
        assert_eq!(start["config"]["model_reasoning_effort"], "high");
        assert_eq!(start["effort"], "high");
        let start_without = thread_start_params("/tmp/work", None, None, None, None);
        assert!(start_without.get("config").is_none());

        // A fork is an aside, not a model switch — its effort is out of scope
        // and stays untouched.
        let fork = thread_fork_params(
            "thread-existing",
            "/tmp/work",
            Some("runtime-model"),
            None,
            None,
        );
        assert!(fork.get("config").is_none());
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
    fn compact_start_request_carries_the_thread_id_and_nothing_else() {
        let params = compact_start_params("thread-existing");
        assert_eq!(params["threadId"], "thread-existing");
        assert_eq!(
            params.as_object().unwrap().len(),
            1,
            "a focus, a target size or a summary instruction would be invented: {params}"
        );
    }

    #[test]
    fn native_compaction_capability_is_discovered_from_protocol_schema() {
        // Read off the same generated `ClientRequest.json` the fork and resume
        // probes read, so a Codex without the verb falls back to a Bridge
        // checkpoint rather than writing a request it will not answer.
        assert!(schema_supports_compact(
            r#"{"method":"thread/compact/start","params":{"$ref":"ThreadCompactStartParams"}}"#
        ));
        assert!(!schema_supports_compact(
            r#"{"method":"thread/compacted","params":{"$ref":"ContextCompactedNotification"}}"#
        ));
        // The method name alone is not the capability: a schema that mentions
        // it without the params type is not one Bridge can call.
        assert!(!schema_supports_compact(r#"{"method":"thread/compact/start"}"#));
    }

    #[test]
    fn resume_request_uses_stored_thread_and_current_enforcement() {
        let params = thread_resume_params(
            "thread-existing",
            "/tmp/work",
            Some("runtime-model"),
            None,
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
            TurnContext {
                session: Some("<bridge-session-context schema=\"1\">frame</bridge-session-context>"),
                credentials: Some("trusted broker capability"),
            },
            None,
        );
        // The session frame leads the turn's own input items, so it is part of
        // the user message and persists in the thread. The user's words follow
        // it, unedited.
        let input = params["input"].as_array().unwrap();
        assert_eq!(input.len(), 2);
        assert_eq!(
            input[0]["text"],
            "<bridge-session-context schema=\"1\">frame</bridge-session-context>"
        );
        assert_eq!(input[0]["type"], "text");
        assert_eq!(input[1]["text"], "verify [secret:sec_reference]");

        // Per-turn credential context keeps its turn-scoped home.
        assert_eq!(
            params["additionalContext"]["bridge.credentials"]["kind"],
            "application"
        );
        assert_eq!(
            params["additionalContext"]["bridge.credentials"]["value"],
            "trusted broker capability"
        );
        assert!(
            params["additionalContext"].get("bridge.session").is_none(),
            "a standing contract must not live in a turn-scoped fragment"
        );
    }

    /// `TurnStartParams` marks eleven fields as applying "for this turn and
    /// subsequent turns". `additionalContext` is not one of them, so anything
    /// the delivery ledger expects the thread to still hold next turn cannot
    /// go there.
    #[test]
    fn a_standing_frame_rides_input_while_per_turn_context_rides_additional_context() {
        let frame_only = turn_start_params(
            "thread",
            "hello",
            TurnContext {
                session: Some("frame"),
                credentials: None,
            },
            None,
        );
        assert_eq!(frame_only["input"].as_array().unwrap().len(), 2);
        assert!(
            frame_only.get("additionalContext").is_none(),
            "no per-turn fragment means no additionalContext at all"
        );

        let credentials_only = turn_start_params(
            "thread",
            "verify",
            TurnContext {
                session: None,
                credentials: Some("trusted broker capability"),
            },
            None,
        );
        let input = credentials_only["input"].as_array().unwrap();
        assert_eq!(input.len(), 1, "no frame means the user text stands alone");
        assert_eq!(input[0]["text"], "verify");
        let additional = credentials_only["additionalContext"].as_object().unwrap();
        assert_eq!(additional.len(), 1);
        assert!(additional.contains_key("bridge.credentials"));
    }

    #[test]
    fn a_blank_context_value_is_absence_not_an_empty_claim() {
        let blank = turn_start_params(
            "thread",
            "verify",
            TurnContext {
                session: Some("   "),
                credentials: Some("  \n "),
            },
            None,
        );
        assert!(blank.get("additionalContext").is_none());
        assert_eq!(blank["input"].as_array().unwrap().len(), 1);

        // A context-free turn is byte-identical to the wire before #528.
        let plain = turn_start_params("thread", "verify", TurnContext::default(), None);
        assert_eq!(
            plain,
            json!({"threadId":"thread","input":[{"type":"text","text":"verify","text_elements":[]}]})
        );
    }

    #[test]
    fn read_only_turn_adds_only_the_assigned_output_root() {
        let policy = json!({
            "type": "workspaceWrite",
            "writableRoots": ["/tmp/bridge-output"],
            "networkAccess": false,
        });
        let params = turn_start_params("thread", "verify", TurnContext::default(), Some(&policy));
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
            on_progress: None,
        })
        .unwrap();
        let mut runtime = started.runtime;
        let mut reader = started.reader;
        runtime
            .start_turn(
                "Reply exactly BRIDGE_SMOKE_OK. Do not use tools.",
                TurnContext::default(),
            )
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
            started
                .runtime
                .start_turn(prompt, TurnContext::default())
                .unwrap();
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
            on_progress: None,
        })
        .unwrap();
        run_turn(&mut started, "Remember this exact token for the next turn: BRIDGE_CODEX_RESUME_8F31. Reply only SAVED.");
        let thread_id = started.runtime.thread_id.clone();
        started.runtime.stop(ShutdownReason::AppShutdown);

        let mut resumed = resume(ResumeRequest {
            fork: false,
            cwd,
            model: None,
            effort: None,
            instructions: None,
            write_mode: None,
            read_only_sandbox: None,
            briefing: None,
            provider_session_id: &thread_id,
            on_progress: None,
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

    #[test]
    fn auth_probe_respects_codex_home() {
        let home = tempfile::tempdir().unwrap();
        let configured = tempfile::tempdir().unwrap();
        std::fs::write(configured.path().join("auth.json"), "opaque").unwrap();
        let environment = crate::capability_projection::CapabilityEnvironment {
            codex_home: Some(configured.path().to_path_buf()),
            ..Default::default()
        };
        assert_eq!(
            auth_state_from_environment(Some(home.path().to_path_buf()), &environment),
            AuthState::SignedIn
        );
    }
}

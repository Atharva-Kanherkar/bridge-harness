//! Pi's native JSONL RPC transport. Credentials and inference belong to Pi.
//! Only full-access execution is advertised; the bundled extension mediates
//! tool authorization and project resources are explicitly disabled.
mod normalize;
mod rpc;
#[cfg(test)]
mod tests;

use crate::{
    adapters::{
        AdapterRuntime, HarnessAdapter, NativeCompaction, ResumeRequest, ShutdownReason,
        StartRequest, StartedAdapter, StartupPhase, TurnContext,
    },
    agent::NormalizedEvent,
    binary,
    delegation::WriteMode,
    model::{
        AdapterDescriptor, AuthState, CapabilityTier, ModelCatalogDiagnostics, ModelCatalogSource,
        ModelOption, SandboxMode,
    },
    model_catalog::{self, CatalogCandidate},
    BridgeError,
};
use serde_json::{json, Value};
use std::{
    any::Any,
    collections::{HashMap, HashSet},
    io::Read,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, RwLock,
    },
    thread,
    time::{Duration, Instant},
};

const READY: &str = "bridge-pi-permissions-v1";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const CAPABILITIES: &[&str] = &[
    "messages",
    "streaming",
    "reasoning",
    "tools",
    "commands",
    "approvals",
    "usage",
    "history",
    "interrupt",
    "steering",
];
pub(crate) const SANDBOXES: &[SandboxMode] = &[SandboxMode::DangerFullAccess];

#[derive(Clone)]
struct Profile {
    executable: PathBuf,
    version: String,
    models: Vec<ModelOption>,
    default_model: Option<String>,
}
fn invalid(message: impl Into<String>) -> BridgeError {
    BridgeError::Invalid(message.into())
}
fn system_executable() -> Result<PathBuf, BridgeError> {
    binary::resolve("pi").ok_or_else(|| invalid("Pi is not installed. Install @earendil-works/pi-coding-agent and configure providers with pi, then refresh Coding agents."))
}

/// --version must have a deadline as well; do not let a wrong PATH executable
/// wedge discovery or a desktop launch. Its output is never a diagnostic.
fn version(executable: &Path, cwd: &str) -> Result<String, BridgeError> {
    let mut command = crate::adapters::supervised_command(executable, ["--version"]);
    binary::hydrate_command_path(&mut command);
    crate::adapters::configure_process_group(&mut command);
    let mut child = command
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut chunk = [0; 1024];
        while let Ok(count) = stdout.read(&mut chunk) {
            if count == 0 {
                break;
            }
            let remaining = 128usize.saturating_sub(bytes.len());
            bytes.extend_from_slice(&chunk[..count.min(remaining)]);
        }
        let _ = tx.send(bytes);
    });
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            _ => break None,
        }
    };
    crate::adapters::terminate_process_group(child.id());
    let _ = child.kill();
    let _ = child.wait();
    if !status.is_some_and(|status| status.success()) {
        return Err(invalid(
            "Pi --version failed or timed out; check the executable on PATH",
        ));
    }
    let bytes = rx
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| invalid("Pi did not close its version output"))?;
    let text = String::from_utf8_lossy(&bytes);
    let value = text.trim().strip_prefix("pi ").unwrap_or(text.trim());
    let parsed = semver::Version::parse(value).map_err(|_| {
        invalid("Pi reported an unreadable version; install the Pi coding-agent CLI")
    })?;
    if parsed.major != 1 || parsed.minor < 1 {
        return Err(invalid("Pi RPC requires Pi 1.1+ with project-trust controls and agent_settled; update the Pi CLI"));
    }
    Ok(parsed.to_string())
}
fn session_directory() -> Result<PathBuf, BridgeError> {
    let agent = std::env::var_os("PI_CODING_AGENT_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".pi/agent")))
        .ok_or_else(|| invalid("Pi session storage needs HOME or PI_CODING_AGENT_DIR"))?;
    let path = agent.join("sessions/bridge");
    std::fs::create_dir_all(&path)?;
    Ok(path.canonicalize()?)
}
fn launch_args(
    extension: &Path,
    session_dir: &Path,
    request: StartRequest<'_>,
    resume: Option<&str>,
    completion: bool,
) -> Vec<String> {
    let mut args = [
        "--mode",
        "rpc",
        "--no-approve",
        "--offline",
        "--no-extensions",
        "--no-mcp",
        "--no-skills",
        "--no-prompt-templates",
        "--no-themes",
        "--no-context-files",
    ]
    .map(str::to_owned)
    .to_vec();
    args.extend([
        "--extension".into(),
        extension.to_string_lossy().into_owned(),
        "--session-dir".into(),
        session_dir.to_string_lossy().into_owned(),
    ]);
    if completion {
        args.extend(["--no-session".into(), "--no-tools".into()]);
    }
    if let Some(resume) = resume {
        args.extend(["--session".into(), resume.into()]);
    }
    if let Some(instructions) = request.instructions.filter(|text| !text.is_empty()) {
        args.extend(["--append-system-prompt".into(), instructions.into()]);
    }
    args
}
fn validate_scope(request: StartRequest<'_>) -> Result<(), BridgeError> {
    if request.briefing.is_some() {
        return Err(invalid(
            "Pi cannot run Bridge briefings; no certified briefing boundary is available",
        ));
    }
    if request.read_only_sandbox.is_some()
        || matches!(
            request.write_mode,
            Some(WriteMode::ReadOnly | WriteMode::Shared | WriteMode::Isolated)
        )
    {
        return Err(invalid("Pi supports only full-access sessions. It cannot enforce a read-only or workspace-write sandbox."));
    }
    Ok(())
}
fn model_identity(model: &Value) -> Result<String, BridgeError> {
    let provider = model["provider"]
        .as_str()
        .filter(|value| {
            !value.is_empty()
                && !value.contains('/')
                && value.len() <= 128
                && !value.chars().any(char::is_control)
        })
        .ok_or_else(|| invalid("Pi catalog contains an invalid provider ID"))?;
    let id = model["id"]
        .as_str()
        .filter(|value| {
            !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
        })
        .ok_or_else(|| invalid("Pi catalog contains an invalid model ID"))?;
    Ok(format!("{provider}/{id}"))
}
fn model_candidates(data: &Value) -> Result<Vec<CatalogCandidate>, BridgeError> {
    let models = data["models"]
        .as_array()
        .ok_or_else(|| invalid("Pi returned an incompatible model catalog"))?;
    if models.len() > model_catalog::MAX_CATALOG_ENTRIES {
        return Err(invalid("Pi model catalog exceeds Bridge's entry limit"));
    }
    let mut ids = HashSet::new();
    models
        .iter()
        .map(|model| {
            let id = model_identity(model)?;
            if !ids.insert(id.clone()) {
                return Err(invalid("Pi returned duplicate provider/model IDs"));
            }
            // Retain no API URLs, headers, keys, or raw model objects.
            Ok(CatalogCandidate::stable(
                id.clone(),
                format!(
                    "{} · {}",
                    model["name"]
                        .as_str()
                        .unwrap_or(model["id"].as_str().unwrap())
                        .chars()
                        .take(160)
                        .collect::<String>(),
                    model["provider"].as_str().unwrap()
                ),
                CapabilityTier::Standard,
                0,
            ))
        })
        .collect()
}
fn select_model(client: &rpc::Client, id: &str) -> Result<Vec<String>, BridgeError> {
    let (provider, model_id) = id
        .split_once('/')
        .ok_or_else(|| invalid("Pi models require a provider-qualified ID"))?;
    let selected = client.command(
        json!({"type":"set_model","provider":provider,"modelId":model_id}),
        COMMAND_TIMEOUT,
    )?;
    if model_identity(&selected)? != id {
        return Err(invalid("Pi selected a different model than requested"));
    }
    let levels = client.command(
        json!({"type":"get_available_thinking_levels"}),
        COMMAND_TIMEOUT,
    )?;
    let levels = levels["levels"]
        .as_array()
        .ok_or_else(|| invalid("Pi did not report its thinking levels"))?;
    let mut seen = HashSet::new();
    levels
        .iter()
        .map(|level| {
            level
                .as_str()
                .filter(|level| {
                    ["off", "minimal", "low", "medium", "high", "xhigh"].contains(level)
                        && seen.insert((*level).to_owned())
                })
                .map(str::to_owned)
                .ok_or_else(|| invalid("Pi reported incompatible thinking levels"))
        })
        .collect()
}

pub struct PiAdapter {
    profile: Arc<RwLock<Option<Result<Profile, String>>>>,
    discovering: Arc<AtomicBool>,
    notify: Option<Arc<dyn Fn() + Send + Sync>>,
    streams: Mutex<HashMap<String, normalize::StreamState>>,
}
impl PiAdapter {
    pub fn new(notify: Option<Arc<dyn Fn() + Send + Sync>>) -> Self {
        let adapter = Self {
            profile: Arc::new(RwLock::new(None)),
            discovering: Arc::new(AtomicBool::new(false)),
            notify,
            streams: Mutex::new(HashMap::new()),
        };
        adapter.refresh_availability();
        adapter
    }
    fn discover(executable: &Path) -> Result<Profile, BridgeError> {
        let directory = tempfile::tempdir()?;
        let cwd = directory.path().to_string_lossy().into_owned();
        let version = version(executable, &cwd)?;
        let (mut runtime, _reader) = PiRuntime::open(
            executable,
            directory.path(),
            StartRequest {
                cwd: &cwd,
                model: None,
                effort: None,
                instructions: None,
                write_mode: None,
                read_only_sandbox: None,
                briefing: None,
                on_progress: None,
            },
            None,
            true,
        )?;
        let state = runtime
            .process
            .client
            .command(json!({"type":"get_state"}), COMMAND_TIMEOUT)?;
        let default_model = state.get("model").map(model_identity).transpose()?;
        let data = runtime
            .process
            .client
            .command(json!({"type":"get_available_models"}), COMMAND_TIMEOUT)?;
        let mut candidates = model_candidates(&data)?;
        let deadline = Instant::now() + Duration::from_secs(60);
        for candidate in &mut candidates {
            if Instant::now() >= deadline {
                return Err(invalid("Pi model discovery exceeded its deadline; reduce the configured catalog and refresh"));
            }
            match select_model(&runtime.process.client, &candidate.id) {
                Ok(levels) => candidate.supported_effort_levels = levels,
                Err(_) => candidate.available = false,
            }
            if Some(candidate.id.as_str()) == default_model.as_deref() {
                candidate.promotion_priority = 1;
            }
        }
        let models = model_catalog::normalize(ModelCatalogSource::RuntimeApi, candidates);
        let default_model = default_model
            .filter(|id| {
                models
                    .iter()
                    .any(|model| model.available && &model.id == id)
            })
            .or_else(|| {
                models
                    .iter()
                    .find(|model| model.available && model.compatible)
                    .map(|model| model.id.clone())
            });
        runtime.stop(ShutdownReason::Completed);
        Ok(Profile {
            executable: executable.into(),
            version,
            models,
            default_model,
        })
    }
    fn launch(
        &self,
        request: StartRequest<'_>,
        resume: Option<&str>,
        completion: bool,
    ) -> Result<StartedAdapter, BridgeError> {
        validate_scope(request)?;

        let cached = self.profile.read().unwrap().clone();
        let profile =
            cached.and_then(Result::ok).ok_or_else(|| {
                invalid(self.descriptor().unavailable_reason.unwrap_or_else(|| {
                    "Pi discovery has not completed; refresh Coding agents".into()
                }))
            })?;
        let model = request.model.or(profile.default_model.as_deref()).ok_or_else(|| invalid("Pi has no configured model; configure a provider with pi and refresh Coding agents"))?;
        if !profile
            .models
            .iter()
            .any(|entry| entry.id == model && entry.available && entry.compatible)
        {
            return Err(invalid("The selected Pi model is unavailable; refresh Coding agents or configure the provider in pi"));
        }
        let directory = session_directory()?;
        if let Some(resume) = resume {
            let path = Path::new(resume).canonicalize().map_err(|_| {
                invalid("Pi's saved session file is missing; restore from a Bridge checkpoint")
            })?;
            if path.parent() != Some(directory.as_path())
                || path.extension().and_then(|value| value.to_str()) != Some("jsonl")
            {
                return Err(invalid(
                    "Pi resume file is outside Bridge's Pi session directory",
                ));
            }
        }
        if let Some(progress) = request.on_progress {
            progress(StartupPhase::Spawning);
        }
        let version = version(&profile.executable, request.cwd)?;
        let (mut runtime, reader) =
            PiRuntime::open(&profile.executable, &directory, request, resume, completion)?;
        let levels = select_model(&runtime.process.client, model)?;
        if let Some(effort) = request.effort {
            if !levels.iter().any(|level| level == effort) {
                return Err(invalid(
                    "The selected Pi model does not support this thinking level",
                ));
            }
            runtime.process.client.command(
                json!({"type":"set_thinking_level","level":effort}),
                COMMAND_TIMEOUT,
            )?;
        }
        let state = runtime
            .process
            .client
            .command(json!({"type":"get_state"}), COMMAND_TIMEOUT)?;
        runtime.images = state["model"]["input"]
            .as_array()
            .is_some_and(|input| input.iter().any(|kind| kind == "image"));
        if model_identity(&state["model"])? != model
            || request
                .effort
                .is_some_and(|effort| state["thinkingLevel"] != effort)
        {
            return Err(invalid(
                "Pi did not apply the requested model/thinking level",
            ));
        }
        if let Some(resume) = resume {
            if runtime.provider_session_id != resume {
                return Err(invalid("Pi did not resume the requested session file"));
            }
        }
        let mut updated = profile.clone();
        updated.version = version;
        if let Some(option) = updated.models.iter_mut().find(|entry| entry.id == model) {
            option.supported_effort_levels = levels;
        }
        *self.profile.write().unwrap() = Some(Ok(updated));
        if let Some(progress) = request.on_progress {
            progress(StartupPhase::SessionOpen);
        }
        Ok(StartedAdapter {
            runtime: Box::new(runtime),
            reader: Box::new(reader),
            startup_messages: Vec::new(),
        })
    }
}
impl HarnessAdapter for PiAdapter {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn descriptor(&self) -> AdapterDescriptor {
        let cached = self.profile.read().unwrap();
        let profile = cached.as_ref().and_then(|result| result.as_ref().ok());
        let reason = cached
            .as_ref()
            .and_then(|result| result.as_ref().err())
            .cloned();
        let available = profile.is_some_and(|profile| {
            profile
                .models
                .iter()
                .any(|model| model.available && model.compatible)
        });
        AdapterDescriptor { id: "pi".into(), label: "Pi".into(), available, auth_state: AuthState::Unknown, version: profile.map(|profile| profile.version.clone()), capabilities: CAPABILITIES.iter().map(|value| (*value).into()).collect(), sandbox_modes: SANDBOXES.to_vec(), unavailable_reason: (!available).then(|| reason.clone().unwrap_or_else(|| if profile.is_some() { "Pi has no usable configured model; configure a provider with pi and refresh Coding agents".into() } else { "Pi discovery has not completed; install/configure pi and refresh Coding agents".into() })), models: profile.map(|profile| profile.models.clone()).unwrap_or_default(), default_model: profile.and_then(|profile| profile.default_model.clone()), model_catalog: ModelCatalogDiagnostics { source: ModelCatalogSource::RuntimeApi, fetched_at: None, expires_at: None, stale: false, last_error: reason } }
    }
    fn start(&self, request: StartRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        self.launch(request, None, false)
    }
    fn start_completion(&self, request: StartRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        self.launch(request, None, true)
    }
    fn resume(&self, request: ResumeRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        if request.fork {
            return Err(invalid("Pi native fork is not supported"));
        }
        self.launch(
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
            false,
        )
    }
    fn supports_native_resume(&self) -> bool {
        true
    }
    fn supports_native_compaction(&self) -> bool {
        true
    }
    fn normalize(&self, value: &Value) -> Vec<NormalizedEvent> {
        let Some(session) = value["piSession"]
            .as_str()
            .filter(|value| !value.is_empty())
        else {
            return vec![];
        };
        self.streams
            .lock()
            .unwrap()
            .entry(session.into())
            .or_default()
            .normalize(&value["event"])
    }
    fn forget_session(&self, id: &str) {
        self.streams.lock().unwrap().remove(id);
    }
    fn refresh_availability(&self) {
        if self.discovering.swap(true, Ordering::AcqRel) {
            return;
        }
        let profile = self.profile.clone();
        let discovering = self.discovering.clone();
        let notify = self.notify.clone();
        thread::spawn(move || {
            let result = system_executable()
                .and_then(|executable| Self::discover(&executable))
                .map_err(|error| error.to_string());
            *profile.write().unwrap() = Some(result);
            discovering.store(false, Ordering::Release);
            if let Some(notify) = notify {
                notify();
            }
        });
    }
    fn prepare_setup(&self) -> Result<(), BridgeError> {
        self.refresh_availability();
        Ok(())
    }
}

struct PiRuntime {
    process: rpc::Process,
    provider_session_id: String,
    current_turn: Arc<Mutex<Option<String>>>,
    _extension: tempfile::TempDir,
    images: bool,
}
impl PiRuntime {
    fn open(
        executable: &Path,
        session_dir: &Path,
        request: StartRequest<'_>,
        resume: Option<&str>,
        completion: bool,
    ) -> Result<(Self, rpc::Reader), BridgeError> {
        let extension = tempfile::tempdir()?;
        let path = extension.path().join("bridge-permissions.mjs");
        std::fs::write(&path, include_str!("pi_adapter/permission.mjs"))?;
        let args = launch_args(&path, session_dir, request, resume, completion);
        let (process, reader) = rpc::Process::spawn(executable, &args, request.cwd)?;
        if let Some(progress) = request.on_progress {
            progress(StartupPhase::Handshake);
        }
        let state = process
            .client
            .command(json!({"type":"get_state"}), COMMAND_TIMEOUT)?;
        if !process.client.ready.load(Ordering::Acquire) {
            return Err(invalid("Pi did not load Bridge's permission extension; refusing to run tools. Update or repair Pi."));
        }
        if state["sessionId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .is_none()
            || state["isStreaming"] != false
        {
            return Err(invalid("Pi returned an incompatible or busy session state"));
        }
        let provider_session_id = state["sessionFile"]
            .as_str()
            .filter(|value| !value.is_empty())
            .or_else(|| state["sessionId"].as_str())
            .unwrap()
            .to_owned();
        if !completion {
            let path = Path::new(&provider_session_id);
            if path.parent() != Some(session_dir)
                || path.extension().and_then(|value| value.to_str()) != Some("jsonl")
            {
                return Err(invalid(
                    "Pi did not open a session in Bridge's explicit session directory",
                ));
            }
        }
        *process.client.session.lock().unwrap() = provider_session_id.clone();
        Ok((
            Self {
                process,
                provider_session_id,
                current_turn: Arc::new(Mutex::new(None)),
                _extension: extension,
                images: state["model"]["input"]
                    .as_array()
                    .is_some_and(|input| input.iter().any(|kind| kind == "image")),
            },
            reader,
        ))
    }
    fn send(
        &self,
        text: &str,
        context: TurnContext<'_>,
        images: Option<&[bridge_protocol::messages::TurnImage]>,
    ) -> Result<(), BridgeError> {
        if context
            .credentials
            .is_some_and(|value| !value.trim().is_empty())
        {
            return Err(invalid("Pi RPC cannot receive Bridge's private credential context. Use credentials configured directly in Pi."));
        }
        // RPC has no hidden trusted-context channel. The session packet is
        // deliberately omitted rather than folded into visible user text.
        if text.trim_start().starts_with(['/', '!']) {
            return Err(invalid("Pi slash commands and direct shell input are unavailable in Bridge. Use plain text or Bridge's compaction control."));
        }
        let mut command = json!({"type":"prompt","message":text,"streamingBehavior":"steer"});
        if let Some(images) = images {
            if !self.images {
                return Err(invalid("The selected Pi model does not accept images"));
            }
            command["images"] = json!(images
                .iter()
                .map(|image| json!({"type":"image","data":image.base64_data,"mimeType":image.media_type}))
                .collect::<Vec<_>>());
        }
        self.process.client.command(command, COMMAND_TIMEOUT)?;
        Ok(())
    }
}
impl AdapterRuntime for PiRuntime {
    fn process_id(&self) -> u32 {
        self.process.child.id()
    }
    fn provider_session_id(&self) -> &str {
        &self.provider_session_id
    }
    fn context_inventory(&self) -> Vec<crate::context_inventory::AdapterContextInventory> {
        crate::context_inventory::adapter_context_inventory_contract(
            "pi",
            crate::context_inventory::ContextLifecyclePhase::PerTurn,
        )
        .unwrap_or_default()
    }
    fn current_turn(&self) -> Arc<Mutex<Option<String>>> {
        self.current_turn.clone()
    }
    fn event_queue_metrics(&self) -> Option<crate::frame_queue::QueueMetricsSnapshot> {
        Some(self.process.metrics.snapshot())
    }
    fn send_turn(&self, text: &str) -> Result<(), BridgeError> {
        self.send(text, TurnContext::default(), None)
    }
    fn send_turn_with_context(
        &self,
        text: &str,
        context: TurnContext<'_>,
    ) -> Result<(), BridgeError> {
        self.send(text, context, None)
    }
    fn supports_trusted_turn_context(&self) -> bool {
        false
    }
    fn supports_images(&self) -> bool {
        self.images
    }
    fn send_turn_with_images(
        &self,
        text: &str,
        context: TurnContext<'_>,
        images: &[bridge_protocol::messages::TurnImage],
    ) -> Result<(), BridgeError> {
        self.send(text, context, Some(images))
    }
    fn supports_active_turn_steering(&self) -> bool {
        true
    }
    fn interrupt(&self) -> Result<(), BridgeError> {
        self.process
            .client
            .command(json!({"type":"clear_queue"}), COMMAND_TIMEOUT)?;
        let pending = self
            .process
            .client
            .permissions
            .lock()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for id in pending {
            self.process
                .client
                .write(&json!({"type":"extension_ui_response","id":id,"cancelled":true}))?;
        }
        self.process
            .client
            .command(json!({"type":"abort"}), COMMAND_TIMEOUT)?;
        self.process.client.permissions.lock().unwrap().clear();
        Ok(())
    }
    fn respond(&self, request_id: Value, decision: &str) -> Result<(), BridgeError> {
        let id = request_id
            .as_str()
            .ok_or_else(|| invalid("Pi permission IDs must be strings"))?;
        if !["accept", "acceptForSession", "decline", "cancel"].contains(&decision) {
            return Err(invalid("Unsupported Pi permission decision"));
        }
        let pending = self
            .process
            .client
            .permissions
            .lock()
            .unwrap()
            .remove(id)
            .ok_or_else(|| invalid("Pi permission request is no longer pending"))?;
        if let Err(error) = self.process.client.write(&json!({"type":"extension_ui_response","id":id,"confirmed":matches!(decision,"accept" | "acceptForSession")})) { self.process.client.permissions.lock().unwrap().insert(id.into(),pending); return Err(error); }
        Ok(())
    }
    fn native_compaction(&self) -> NativeCompaction {
        NativeCompaction::WithFocus
    }
    fn compact_native(&self, focus: Option<&str>) -> Result<(), BridgeError> {
        let mut command = json!({"type":"compact"});
        if let Some(focus) = focus {
            command["customInstructions"] = json!(focus);
        }
        self.process.client.command_async(command)
    }
    fn failure_context(&mut self) -> Option<String> {
        let status = self.process.child.try_wait().ok().flatten();
        (status.is_some() || self.process.diagnostic.load(Ordering::Relaxed)).then(|| {
            format!(
                "Pi process {}. {}Run pi directly to inspect provider diagnostics.",
                status
                    .map(|status| status.to_string())
                    .unwrap_or_else(|| "has emitted diagnostics".into()),
                if self.process.diagnostic.load(Ordering::Relaxed) {
                    "Stderr was withheld to protect provider credentials. "
                } else {
                    ""
                }
            )
        })
    }
    fn stop(&mut self, _reason: ShutdownReason) {
        self.process.stop();
    }
}

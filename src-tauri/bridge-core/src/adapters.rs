use crate::{
    briefing_policy::BriefingRuntimePolicy,
    agent, claude_adapter, codex_adapter, cursor_adapter,
    delegation::WriteMode,
    model::{AdapterDescriptor, CapabilityTier, ModelOption, SandboxMode},
    opencode_adapter,
    worker_sandbox::ReadOnlySandbox,
    BridgeError,
};
use serde_json::Value;
use std::{
    any::Any,
    collections::HashMap,
    io::BufRead,
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Mutex, RwLock},
    thread,
    time::Duration,
};

pub trait AdapterRuntime: Send {
    fn process_id(&self) -> u32;
    fn provider_session_id(&self) -> &str;
    /// Live event-queue pressure for diagnostics; `None` for providers
    /// without a bounded frame queue.
    fn event_queue_metrics(&self) -> Option<crate::frame_queue::QueueMetricsSnapshot> {
        None
    }
    fn current_turn(&self) -> Arc<Mutex<Option<String>>>;
    /// Adapter-owned observations recorded at the provider injection points.
    /// The inventory is deliberately separate from Bridge prompt accounting.
    fn context_inventory(&self) -> Vec<crate::context_inventory::AdapterContextInventory> {
        Vec::new()
    }
    fn send_turn(&self, text: &str) -> Result<(), BridgeError>;
    /// Send a user turn with trusted, application-owned context that must not
    /// be folded into the visible user message. Providers that cannot attach
    /// per-turn context retain their startup instructions and send normally.
    fn send_turn_with_context(
        &self,
        text: &str,
        _application_context: &str,
    ) -> Result<(), BridgeError> {
        self.send_turn(text)
    }
    /// Whether this provider can receive base64 image content blocks beside
    /// the user's text. Kept in step with a `send_turn_with_images` override:
    /// advertising `true` while keeping the default delivery would route
    /// image turns straight into the refusal below.
    fn supports_images(&self) -> bool {
        false
    }
    /// Send a turn whose message carries image attachments as provider-shaped
    /// content. `application_context` matches `send_turn_with_context`'s
    /// trusted-context contract. The default errs rather than dropping: a
    /// provider without the capability must fail loudly at the routing seam,
    /// never quietly on the wire.
    fn send_turn_with_images(
        &self,
        _text: &str,
        _application_context: Option<&str>,
        _images: &[bridge_protocol::messages::TurnImage],
    ) -> Result<(), BridgeError> {
        Err(BridgeError::Invalid(
            "This provider does not accept image attachments".into(),
        ))
    }
    /// Whether this provider can take a user message while one of its own
    /// turns is still running, and fold it into that turn.
    ///
    /// The default is `false` on purpose: for most providers a second
    /// `turn/start` against a live turn is either rejected or silently races the
    /// one in flight, so Bridge queues instead of guessing. A provider only
    /// advertises `true` where its transport is genuinely a stream of user
    /// messages the running turn consumes.
    ///
    /// Keep this in step with the `steering` capability on the harness
    /// descriptor — that string is how the UI knows to offer Steer instead of
    /// Queue before the input is submitted.
    fn supports_active_turn_steering(&self) -> bool {
        false
    }
    fn interrupt(&self) -> Result<(), BridgeError>;
    fn respond(&self, request_id: Value, decision: &str) -> Result<(), BridgeError>;
    /// Answer a pending question this provider raised on its own channel —
    /// distinct from `respond`, which grants or denies a permission decision.
    /// `answers` is provider-shaped (OpenCode expects one array of chosen
    /// values per question asked); callers that build it own that shape.
    ///
    /// The default errs: only a provider that actually raises question-shaped
    /// requests overrides this, so routing a typed reply here for any other
    /// provider fails loudly instead of silently doing nothing.
    fn answer_question(&self, _request_id: Value, _answers: Value) -> Result<(), BridgeError> {
        Err(BridgeError::Invalid(
            "This provider does not raise question-shaped requests".into(),
        ))
    }
    /// Reject a pending question this provider raised, as a dedicated channel
    /// from `respond`'s permission decline. See `answer_question`.
    fn reject_question(&self, _request_id: Value) -> Result<(), BridgeError> {
        Err(BridgeError::Invalid(
            "This provider does not raise question-shaped requests".into(),
        ))
    }
    /// Ask the provider to report current subscription rate-limit usage.
    /// The response arrives asynchronously on the session's event stream.
    /// Providers without an on-demand usage query keep the default no-op.
    fn read_usage(&self) -> Result<(), BridgeError> {
        Ok(())
    }
    /// Why the provider process died, once it has: exit status plus a bounded
    /// stderr tail. `None` while it is still running or when nothing useful
    /// was captured. Supervisors attach this to the synthetic failure they
    /// report when a worker exits without a typed result — the difference
    /// between "ended without reporting" and the provider's actual error.
    fn failure_context(&mut self) -> Option<String> {
        None
    }
    fn stop(&mut self, reason: ShutdownReason);
}

/// How much of a provider's stderr is retained for failure reporting. Enough
/// for a CLI's final error paragraph; never an unbounded transcript.
const STDERR_TAIL_LINES: usize = 30;
const STDERR_LINE_MAX_BYTES: usize = 500;

/// A bounded rolling tail of a child process's stderr, filled by a detached
/// reader thread so the pipe never backpressures the provider.
#[derive(Clone, Default)]
pub struct StderrTail {
    lines: Arc<Mutex<std::collections::VecDeque<String>>>,
}

impl StderrTail {
    /// Start capturing `child`'s stderr, if it was piped. Always returns a
    /// tail handle — an empty one when there is nothing to read.
    pub fn capture(child: &mut std::process::Child) -> StderrTail {
        let tail = StderrTail::default();
        let Some(stderr) = child.stderr.take() else {
            return tail;
        };
        let lines = tail.lines.clone();
        let _ = thread::Builder::new()
            .name("adapter-stderr-tail".into())
            .spawn(move || {
                let reader = std::io::BufReader::new(stderr);
                for line in reader.lines() {
                    let Ok(mut line) = line else { break };
                    if line.trim().is_empty() {
                        continue;
                    }
                    if line.len() > STDERR_LINE_MAX_BYTES {
                        let mut cut = STDERR_LINE_MAX_BYTES;
                        while !line.is_char_boundary(cut) {
                            cut -= 1;
                        }
                        line.truncate(cut);
                        line.push('…');
                    }
                    let mut lines = lines.lock().unwrap();
                    if lines.len() == STDERR_TAIL_LINES {
                        lines.pop_front();
                    }
                    lines.push_back(line);
                }
            });
        tail
    }

    pub fn snapshot(&self) -> Option<String> {
        let lines = self.lines.lock().unwrap();
        if lines.is_empty() {
            return None;
        }
        Some(lines.iter().cloned().collect::<Vec<_>>().join("\n"))
    }
}

/// The standard [`AdapterRuntime::failure_context`] body for process-backed
/// runtimes: exit status (when the child has exited) plus the stderr tail.
pub fn process_failure_context(
    child: &mut std::process::Child,
    stderr_tail: &StderrTail,
) -> Option<String> {
    let status = match child.try_wait() {
        Ok(Some(status)) => Some(status.to_string()),
        _ => None,
    };
    let tail = stderr_tail.snapshot();
    match (status, tail) {
        (Some(status), Some(tail)) => {
            Some(format!("Provider process {status}. Stderr tail:\n{tail}"))
        }
        (Some(status), None) => Some(format!("Provider process {status} with no stderr output")),
        (None, Some(tail)) => Some(format!("Provider stderr tail:\n{tail}")),
        (None, None) => None,
    }
}

pub const PARENT_WATCHDOG_DISABLE_ENV: &str = "BRIDGE_DISABLE_PARENT_WATCHDOG";

/// Kills the wrapped child when the supervisor that spawned it dies. `Drop`
/// never runs after SIGKILL, a crash, or an aborted test binary, and boot
/// recovery only helps once something boots again — this monitor closes the
/// window in between by polling its own parentage and tearing the child down
/// the moment it is re-parented to init.
// The wrapper is its own process-group leader (configure_process_group runs
// on it), so `-$$` names the whole group: the child and anything it forked.
// Killing only `$child` would leave forked helpers as the very PID-1 orphans
// this monitor exists to prevent. TERM is ignored first so the group signal
// does not interrupt the wrapper's own escalation.
#[cfg(unix)]
const PARENT_WATCHDOG_SCRIPT: &str = r#"cmd="$1"; shift
"$cmd" "$@" &
child=$!
trap 'trap "" TERM INT; kill -TERM -- -$$ 2>/dev/null' TERM INT
while kill -0 "$child" 2>/dev/null; do
  ppid=$(ps -o ppid= -p $$ 2>/dev/null | tr -d ' ')
  if [ -z "$ppid" ] || [ "$ppid" -le 1 ]; then
    trap '' TERM
    kill -TERM -- -$$ 2>/dev/null
    sleep 2
    kill -KILL -- -$$ 2>/dev/null
    exit 143
  fi
  sleep 2 &
  wait $! 2>/dev/null
done
wait "$child""#;

/// A `Command` for `executable` wrapped in the parent-death watchdog. The
/// wrapper shares the child's process group, so group termination and the
/// existing identity/tracking primitives keep working against the returned
/// process id; the child's exit status propagates through the wrapper.
#[cfg(unix)]
pub fn supervised_command(executable: &Path, args: &[&str]) -> Command {
    if std::env::var_os(PARENT_WATCHDOG_DISABLE_ENV).is_some() {
        let mut command = Command::new(executable);
        command.args(args);
        return command;
    }
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(PARENT_WATCHDOG_SCRIPT)
        .arg("bridge-watchdog")
        .arg(executable);
    command.args(args);
    command
}

#[cfg(not(unix))]
pub fn supervised_command(executable: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(executable);
    command.args(args);
    command
}

#[cfg(unix)]
pub fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(unix))]
pub fn configure_process_group(_command: &mut Command) {}

pub fn process_identity(pid: u32) -> Option<String> {
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "lstart=", "-o", "comm="])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let identity = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && !identity.is_empty()).then_some(identity)
}

fn process_group_is_running(pid: u32) -> bool {
    let output = Command::new("ps")
        .args(["-ax", "-o", "pgid=", "-o", "stat="])
        .stderr(Stdio::null())
        .output();
    let Ok(output) = output else {
        return false;
    };
    output.status.success()
        && String::from_utf8_lossy(&output.stdout).lines().any(|line| {
            let mut fields = line.split_whitespace();
            fields.next().and_then(|value| value.parse::<u32>().ok()) == Some(pid)
                && fields.next().is_some_and(|state| !state.starts_with('Z'))
        })
}

#[cfg(unix)]
pub fn terminate_process_group(pid: u32) -> bool {
    let target = format!("-{pid}");
    let signal = |value: &str| {
        Command::new("kill")
            .args([value, &target])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    };
    let alive = || process_group_is_running(pid);
    let _ = signal("-TERM");
    for _ in 0..20 {
        if !alive() {
            return true;
        }
        thread::sleep(Duration::from_millis(25));
    }
    let _ = signal("-KILL");
    for _ in 0..20 {
        if !alive() {
            return true;
        }
        thread::sleep(Duration::from_millis(25));
    }
    !alive()
}

#[cfg(not(unix))]
pub fn terminate_process_group(pid: u32) -> bool {
    Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownReason {
    UserStopped,
    UserCancelled,
    Replaced,
    Completed,
    Failed,
    AppShutdown,
}

impl ShutdownReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserStopped => "user_stopped",
            Self::UserCancelled => "user_cancelled",
            Self::Replaced => "replaced",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::AppShutdown => "app_shutdown",
        }
    }
}

/// A cold-start phase observed at a real launch boundary — never emitted on a
/// timer, and never emitted for a harness where the boundary was not actually
/// reached. See [`crate::events::CoreEvent::SessionStartup`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupPhase {
    /// The provider's child process is about to be spawned (or has just been).
    Spawning,
    /// Waiting on the provider to become responsive: an OpenCode health poll,
    /// a Codex `initialize` round trip, the Claude Node sidecar booting.
    Handshake,
    /// The provider session itself (create or resume) has completed.
    SessionOpen,
}

impl StartupPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Spawning => "spawning",
            Self::Handshake => "handshake",
            Self::SessionOpen => "session_open",
        }
    }
}

/// A launch-progress sink. Adapters call it at the real boundaries they
/// observe; callers with nothing to narrate (worker delegation, briefings,
/// suggestions) pass `None` rather than a no-op closure.
pub type StartupProgress<'a> = &'a dyn Fn(StartupPhase);

#[derive(Clone, Copy)]
pub struct StartRequest<'a> {
    pub cwd: &'a str,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub instructions: Option<&'a str>,
    pub write_mode: Option<WriteMode>,
    pub read_only_sandbox: Option<&'a ReadOnlySandbox>,
    /// Briefing authority, when this session is a briefing run.
    ///
    /// A separate axis from `write_mode`, not a rung of it: see
    /// [`crate::briefing_policy`]. Stated at every start site rather than
    /// defaulted, because an adapter that silently ignores it would run a
    /// briefing with a coding agent's tools.
    pub briefing: Option<&'a BriefingRuntimePolicy>,
    /// See [`StartupProgress`].
    pub on_progress: Option<StartupProgress<'a>>,
}

// Manual, not derived: `on_progress` is a `&dyn Fn`, which has no `Debug` impl
// to derive against. Everything else prints as it always did.
impl std::fmt::Debug for StartRequest<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StartRequest")
            .field("cwd", &self.cwd)
            .field("model", &self.model)
            .field("effort", &self.effort)
            .field("instructions", &self.instructions)
            .field("write_mode", &self.write_mode)
            .field("read_only_sandbox", &self.read_only_sandbox)
            .field("briefing", &self.briefing)
            .field("on_progress", &self.on_progress.map(|_| "<fn>"))
            .finish()
    }
}

#[derive(Clone, Copy)]
pub struct ResumeRequest<'a> {
    pub provider_session_id: &'a str,
    pub cwd: &'a str,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub instructions: Option<&'a str>,
    pub write_mode: Option<WriteMode>,
    pub read_only_sandbox: Option<&'a ReadOnlySandbox>,
    /// See [`StartRequest::briefing`].
    pub briefing: Option<&'a BriefingRuntimePolicy>,
    /// See [`StartupProgress`].
    pub on_progress: Option<StartupProgress<'a>>,
}

impl std::fmt::Debug for ResumeRequest<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResumeRequest")
            .field("provider_session_id", &self.provider_session_id)
            .field("cwd", &self.cwd)
            .field("model", &self.model)
            .field("effort", &self.effort)
            .field("instructions", &self.instructions)
            .field("write_mode", &self.write_mode)
            .field("read_only_sandbox", &self.read_only_sandbox)
            .field("briefing", &self.briefing)
            .field("on_progress", &self.on_progress.map(|_| "<fn>"))
            .finish()
    }
}

pub struct StartedAdapter {
    pub runtime: Box<dyn AdapterRuntime>,
    pub reader: Box<dyn BufRead + Send>,
    pub startup_messages: Vec<Value>,
}

pub trait HarnessAdapter: Send + Sync + Any {
    fn as_any(&self) -> &dyn Any;
    fn descriptor(&self) -> AdapterDescriptor;
    fn start(&self, request: StartRequest<'_>) -> Result<StartedAdapter, BridgeError>;
    fn resume(&self, request: ResumeRequest<'_>) -> Result<StartedAdapter, BridgeError>;
    fn supports_native_resume(&self) -> bool;
    fn normalize(&self, value: &Value) -> Vec<agent::NormalizedEvent>;
    /// Drop any normalization state kept for `provider_session_id`. Called
    /// when the session's runtime is gone; adapters without per-session state
    /// ignore it.
    fn forget_session(&self, _provider_session_id: &str) {}
}

pub struct AdapterRegistry {
    adapters: HashMap<String, Box<dyn HarnessAdapter>>,
}

fn model_options(items: &[(&str, &str, CapabilityTier, bool)]) -> Vec<ModelOption> {
    items
        .iter()
        .map(|(id, label, tier, default_for_tier)| ModelOption {
            id: (*id).into(),
            label: (*label).into(),
            tier: *tier,
            default_for_tier: *default_for_tier,
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelResolution {
    pub requested_tier: CapabilityTier,
    pub actual_model: String,
    pub warning: Option<String>,
}

impl AdapterRegistry {
    /// A registry with no adapters and no background discovery. For hosts and
    /// tests that need a `BridgeCore` without spawning provider processes.
    pub fn empty() -> Self {
        Self {
            adapters: HashMap::new(),
        }
    }

    pub fn built_in() -> Result<Self, BridgeError> {
        Self::built_in_with_opencode(opencode_adapter::OpenCodeSettings::default())
    }

    pub fn built_in_with_opencode(
        opencode_settings: opencode_adapter::OpenCodeSettings,
    ) -> Result<Self, BridgeError> {
        Self::built_in_with_opencode_notify(opencode_settings, None)
    }

    /// `on_opencode_discovered` fires once the background OpenCode catalog
    /// discovery finishes (successfully or not), so the host can tell the
    /// frontend to re-read adapter availability.
    pub fn built_in_with_opencode_notify(
        opencode_settings: opencode_adapter::OpenCodeSettings,
        on_opencode_discovered: Option<Box<dyn FnOnce() + Send>>,
    ) -> Result<Self, BridgeError> {
        let mut registry = Self {
            adapters: HashMap::new(),
        };
        registry.register(Box::new(CodexAdapter))?;
        registry.register(Box::new(ClaudeAdapter {
            streams: Mutex::new(HashMap::new()),
        }))?;
        registry.register(Box::new(OpenCodeAdapter::new(
            opencode_settings,
            on_opencode_discovered,
        )))?;
        // Cursor discovers itself the same way, and for a stronger reason: its
        // protocol support cannot be read off the filesystem and has to be
        // proved with a handshake, which is not something application setup can
        // wait on.
        registry.register(Box::new(cursor_adapter::CursorAdapter::new(None)))?;
        Ok(registry)
    }

    pub fn register(&mut self, adapter: Box<dyn HarnessAdapter>) -> Result<(), BridgeError> {
        let descriptor = adapter.descriptor();
        if descriptor.id.trim().is_empty() {
            return Err(BridgeError::Invalid("Adapter id cannot be empty".into()));
        }
        if self.adapters.contains_key(&descriptor.id) {
            return Err(BridgeError::Invalid(format!(
                "Duplicate adapter id: {}",
                descriptor.id
            )));
        }
        self.adapters.insert(descriptor.id, adapter);
        Ok(())
    }

    pub fn descriptors(&self) -> Vec<AdapterDescriptor> {
        let mut descriptors: Vec<_> = self
            .adapters
            .values()
            .map(|adapter| adapter.descriptor())
            .collect();
        descriptors.sort_by(|a, b| a.id.cmp(&b.id));
        descriptors
    }

    pub fn start(
        &self,
        id: &str,
        request: StartRequest<'_>,
    ) -> Result<StartedAdapter, BridgeError> {
        let adapter = self.adapters.get(id).ok_or_else(|| {
            BridgeError::Invalid(format!("No structured adapter is registered for {id}"))
        })?;
        let descriptor = adapter.descriptor();
        if !descriptor.available {
            return Err(BridgeError::Invalid(
                descriptor
                    .unavailable_reason
                    .unwrap_or_else(|| format!("{} is unavailable", descriptor.label)),
            ));
        }
        adapter.start(request)
    }

    pub fn resume(
        &self,
        id: &str,
        request: ResumeRequest<'_>,
    ) -> Result<StartedAdapter, BridgeError> {
        let adapter = self.adapters.get(id).ok_or_else(|| {
            BridgeError::Invalid(format!("No structured adapter is registered for {id}"))
        })?;
        if !adapter.supports_native_resume() {
            return Err(BridgeError::Invalid(format!(
                "Adapter {id} does not support native resume"
            )));
        }
        adapter.resume(request)
    }

    pub fn supports_native_resume(&self, id: &str) -> bool {
        self.adapters
            .get(id)
            .is_some_and(|adapter| adapter.supports_native_resume())
    }

    pub fn normalize(&self, id: &str, value: &Value) -> Vec<agent::NormalizedEvent> {
        self.adapters
            .get(id)
            .map(|adapter| adapter.normalize(value))
            .unwrap_or_default()
    }

    pub fn forget_session(&self, id: &str, provider_session_id: &str) {
        if let Some(adapter) = self.adapters.get(id) {
            adapter.forget_session(provider_session_id);
        }
    }

    pub fn refresh_opencode(
        &self,
        settings: opencode_adapter::OpenCodeSettings,
        directory: &str,
    ) -> Result<opencode_adapter::OpenCodeCatalog, BridgeError> {
        self.opencode_adapter()?.refresh(settings, directory)
    }

    pub fn opencode_settings(&self) -> Result<opencode_adapter::OpenCodeSettings, BridgeError> {
        Ok(self.opencode_adapter()?.settings())
    }

    pub fn set_opencode_provider_api_key(
        &self,
        directory: &str,
        provider_id: &str,
        api_key: &str,
    ) -> Result<opencode_adapter::OpenCodeCatalog, BridgeError> {
        let adapter = self.opencode_adapter()?;
        let catalog = opencode_adapter::set_provider_api_key(
            &adapter.settings(),
            directory,
            provider_id,
            api_key,
        )?;
        adapter.replace_catalog(catalog.clone());
        Ok(catalog)
    }

    pub fn remove_opencode_provider_auth(
        &self,
        directory: &str,
        provider_id: &str,
    ) -> Result<opencode_adapter::OpenCodeCatalog, BridgeError> {
        let adapter = self.opencode_adapter()?;
        let catalog =
            opencode_adapter::remove_provider_auth(&adapter.settings(), directory, provider_id)?;
        adapter.replace_catalog(catalog.clone());
        Ok(catalog)
    }

    fn opencode_adapter(&self) -> Result<&OpenCodeAdapter, BridgeError> {
        self.adapters
            .get("opencode")
            .and_then(|adapter| adapter.as_any().downcast_ref::<OpenCodeAdapter>())
            .ok_or_else(|| BridgeError::Invalid("OpenCode adapter is not registered".into()))
    }

    pub fn resolve_model(
        &self,
        id: &str,
        tier: CapabilityTier,
        model_hint: Option<&str>,
    ) -> Result<ModelResolution, BridgeError> {
        let descriptor = self
            .adapters
            .get(id)
            .ok_or_else(|| {
                BridgeError::Invalid(format!("No structured adapter is registered for {id}"))
            })?
            .descriptor();
        let tier_default = descriptor
            .models
            .iter()
            .find(|model| model.tier == tier && model.default_for_tier)
            .or_else(|| descriptor.models.iter().find(|model| model.tier == tier))
            .ok_or_else(|| {
                BridgeError::Invalid(format!(
                    "Adapter {id} does not advertise a {} capability model",
                    tier.as_str()
                ))
            })?;
        let hinted = model_hint.and_then(|hint| {
            descriptor
                .models
                .iter()
                .find(|model| model.id.eq_ignore_ascii_case(hint.trim()))
        });
        let selected = hinted
            .filter(|model| model.tier == tier)
            .unwrap_or(tier_default);
        let warning = model_hint.and_then(|hint| {
            (hinted.is_none() || hinted.is_some_and(|model| model.tier != tier)).then(|| {
                format!(
                    "Model hint {hint:?} is unknown or outside tier {}; using {}",
                    tier.as_str(),
                    tier_default.id
                )
            })
        });
        Ok(ModelResolution {
            requested_tier: tier,
            actual_model: selected.id.clone(),
            warning,
        })
    }
}

struct OpenCodeAdapter {
    streams: Mutex<HashMap<String, agent::OpenCodeStreamState>>,
    settings: RwLock<opencode_adapter::OpenCodeSettings>,
    catalog: Arc<RwLock<Option<opencode_adapter::OpenCodeCatalog>>>,
    catalog_error: Arc<RwLock<Option<String>>>,
}
impl OpenCodeAdapter {
    fn new(
        settings: opencode_adapter::OpenCodeSettings,
        on_discovered: Option<Box<dyn FnOnce() + Send>>,
    ) -> Self {
        let adapter = Self {
            streams: Mutex::new(HashMap::new()),
            settings: RwLock::new(settings.clone()),
            catalog: Arc::new(RwLock::new(None)),
            catalog_error: Arc::new(RwLock::new(None)),
        };
        // Discovery spawns an OpenCode server and can take tens of seconds, and
        // new() runs during app setup — do the initial catalog load off-thread.
        let catalog = adapter.catalog.clone();
        let catalog_error = adapter.catalog_error.clone();
        let directory = std::env::current_dir()
            .ok()
            .and_then(|path| path.to_str().map(str::to_owned))
            .unwrap_or_else(|| ".".into());
        let _ = std::thread::Builder::new()
            .name("opencode-discover".into())
            .spawn(move || {
                match opencode_adapter::discover(&settings, &directory) {
                    Ok(result) => {
                        *catalog.write().unwrap() = Some(result);
                        *catalog_error.write().unwrap() = None;
                    }
                    Err(error) => {
                        *catalog_error.write().unwrap() = Some(error.to_string());
                    }
                }
                if let Some(notify) = on_discovered {
                    notify();
                }
            });
        adapter
    }

    fn refresh(
        &self,
        settings: opencode_adapter::OpenCodeSettings,
        directory: &str,
    ) -> Result<opencode_adapter::OpenCodeCatalog, BridgeError> {
        *self.settings.write().unwrap() = settings.clone();
        match opencode_adapter::discover(&settings, directory) {
            Ok(catalog) => {
                *self.catalog.write().unwrap() = Some(catalog.clone());
                *self.catalog_error.write().unwrap() = None;
                Ok(catalog)
            }
            Err(error) => {
                // Keep the last known-good catalog so a transient discovery
                // failure does not degrade a working setup; the error is
                // surfaced alongside it.
                *self.catalog_error.write().unwrap() = Some(error.to_string());
                Err(error)
            }
        }
    }

    fn settings(&self) -> opencode_adapter::OpenCodeSettings {
        self.settings.read().unwrap().clone()
    }

    fn ensure_model_is_selectable(&self, model: Option<&str>) -> Result<(), BridgeError> {
        let Some(model) = model else {
            return Ok(());
        };
        let selectable = self
            .descriptor()
            .models
            .into_iter()
            .any(|option| option.id == model);
        selectable.then_some(()).ok_or_else(|| {
            BridgeError::Invalid(format!(
                "OpenCode model {model:?} is not exposed by a connected provider or is hidden"
            ))
        })
    }

    fn replace_catalog(&self, catalog: opencode_adapter::OpenCodeCatalog) {
        *self.catalog.write().unwrap() = Some(catalog);
        *self.catalog_error.write().unwrap() = None;
    }
}
impl HarnessAdapter for OpenCodeAdapter {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn descriptor(&self) -> AdapterDescriptor {
        let catalog = self.catalog.read().unwrap().clone();
        let models = catalog
            .as_ref()
            .map(|catalog| {
                opencode_adapter::model_options(
                    catalog,
                    &self.settings.read().unwrap().visible_models,
                )
            })
            .unwrap_or_default();
        let default_model = models
            .iter()
            .find(|model| model.tier == CapabilityTier::Standard && model.default_for_tier)
            .or_else(|| models.iter().find(|model| model.default_for_tier))
            .map(|model| model.id.clone());
        let available = catalog.is_some() && !models.is_empty();
        // Every unavailable state names a reason: install status must be
        // distinguishable from auth status, and "unavailable" with no reason
        // reads as a signed-out problem to the usage widget.
        let unavailable_reason = if available {
            None
        } else if catalog.is_some() {
            Some("OpenCode has no connected provider models selected".into())
        } else if let Some(error) = self.catalog_error.read().unwrap().clone() {
            Some(error)
        } else if crate::binary::resolve("opencode").is_none() {
            Some("OpenCode binary is not installed".into())
        } else {
            Some("OpenCode has not finished starting".into())
        };
        AdapterDescriptor {
            id: "opencode".into(),
            label: "OpenCode".into(),
            available,
            auth_state: opencode_adapter::auth_state(),
            version: catalog.as_ref().map(|catalog| catalog.version.clone()),
            capabilities: [
                "messages",
                "streaming",
                "reasoning",
                "plans",
                "tools",
                "commands",
                "file_changes",
                "approvals",
                "usage",
                "history",
                "interrupt",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            // OpenCode drives a localhost HTTP server; the read-only worker
            // sandbox is offline, so a read-only OpenCode worker can never
            // start. Declaring that here lets the router exclude the route
            // before a worker session exists.
            sandbox_modes: vec![SandboxMode::WorkspaceWrite, SandboxMode::DangerFullAccess],
            unavailable_reason,
            models,
            default_model,
        }
    }
    fn start(&self, request: StartRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        self.ensure_model_is_selectable(request.model)?;
        let settings = self.settings();
        let started = opencode_adapter::start_with_settings(request, &settings)?;
        Ok(StartedAdapter {
            runtime: Box::new(started.runtime),
            reader: Box::new(started.reader),
            startup_messages: started.startup_messages,
        })
    }
    fn resume(&self, request: ResumeRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        self.ensure_model_is_selectable(request.model)?;
        let settings = self.settings();
        let started = opencode_adapter::resume_with_settings(request, &settings)?;
        Ok(StartedAdapter {
            runtime: Box::new(started.runtime),
            reader: Box::new(started.reader),
            startup_messages: started.startup_messages,
        })
    }
    fn supports_native_resume(&self) -> bool {
        self.catalog.read().unwrap().is_some()
    }
    fn normalize(&self, value: &Value) -> Vec<agent::NormalizedEvent> {
        let session_key = value
            .pointer("/properties/sessionID")
            .and_then(Value::as_str)
            .unwrap_or("default")
            .to_owned();
        let mut streams = self.streams.lock().unwrap();
        let state = streams.entry(session_key).or_default();
        agent::normalize_opencode_message_with_state(value, state)
    }
    fn forget_session(&self, provider_session_id: &str) {
        // "default" aggregates events that arrive without a session id;
        // per-session teardown must not evict it.
        if provider_session_id == "default" {
            return;
        }
        self.streams.lock().unwrap().remove(provider_session_id);
    }
}

struct CodexAdapter;
impl HarnessAdapter for CodexAdapter {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn descriptor(&self) -> AdapterDescriptor {
        let version = codex_adapter::binary_version();
        AdapterDescriptor {
            id: "codex".into(),
            label: "Codex".into(),
            available: version.is_some(),
            auth_state: codex_adapter::auth_state(),
            version,
            capabilities: [
                "messages",
                "streaming",
                "reasoning",
                "plans",
                "tools",
                "commands",
                "file_changes",
                "approvals",
                "usage",
                "history",
                "interrupt",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            sandbox_modes: SandboxMode::ALL.to_vec(),
            unavailable_reason: codex_adapter::resolve_runtime()
                .is_none()
                .then(|| "Codex binary is not installed".into()),
            models: model_options(&[
                ("gpt-5.6-luna", "GPT Luna", CapabilityTier::Fast, true),
                ("gpt-5.6-terra", "GPT Terra", CapabilityTier::Standard, true),
                ("gpt-5.6-sol", "GPT Sol", CapabilityTier::Strong, true),
                (
                    "gpt-5.3-codex",
                    "GPT-5.3 Codex",
                    CapabilityTier::Standard,
                    false,
                ),
            ]),
            default_model: Some("gpt-5.6-luna".into()),
        }
    }
    fn start(&self, request: StartRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        let started = codex_adapter::start(request)?;
        Ok(StartedAdapter {
            runtime: Box::new(started.runtime),
            reader: Box::new(started.reader),
            startup_messages: started.startup_messages,
        })
    }
    fn resume(&self, request: ResumeRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        let started = codex_adapter::resume(request)?;
        Ok(StartedAdapter {
            runtime: Box::new(started.runtime),
            reader: Box::new(started.reader),
            startup_messages: started.startup_messages,
        })
    }
    fn supports_native_resume(&self) -> bool {
        codex_adapter::supports_native_resume()
    }
    fn normalize(&self, value: &Value) -> Vec<agent::NormalizedEvent> {
        if value.get("id").is_some() && value.get("method").is_some() {
            agent::normalize_codex_request(value).into_iter().collect()
        } else {
            agent::normalize_codex_message(value)
        }
    }
}

struct ClaudeAdapter {
    streams: Mutex<HashMap<String, agent::ClaudeStreamState>>,
}
impl HarnessAdapter for ClaudeAdapter {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn descriptor(&self) -> AdapterDescriptor {
        let version = claude_adapter::binary_version();
        AdapterDescriptor {
            id: "claude".into(),
            label: "Claude Code".into(),
            available: version.is_some(),
            auth_state: claude_adapter::auth_state(),
            version,
            capabilities: [
                "messages",
                "streaming",
                "reasoning",
                "tools",
                "commands",
                "approvals",
                "usage",
                "interrupt",
                // The sidecar drives one streaming-input query, so a user
                // message written mid-turn is consumed by the turn in flight
                // rather than starting a second one.
                "steering",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            sandbox_modes: SandboxMode::ALL.to_vec(),
            unavailable_reason: claude_adapter::unavailable_reason(),
            models: model_options(&[
                ("haiku", "Claude Haiku", CapabilityTier::Fast, true),
                ("sonnet", "Claude Sonnet", CapabilityTier::Standard, true),
                ("opus", "Claude Opus", CapabilityTier::Strong, false),
                ("fable", "Claude Fable", CapabilityTier::Strong, true),
            ]),
            default_model: Some("sonnet".into()),
        }
    }
    fn start(&self, request: StartRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        let started = claude_adapter::start(request)?;
        Ok(StartedAdapter {
            runtime: Box::new(started.runtime),
            reader: Box::new(started.reader),
            startup_messages: started.startup_messages,
        })
    }
    fn resume(&self, request: ResumeRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        let started = claude_adapter::resume(request)?;
        Ok(StartedAdapter {
            runtime: Box::new(started.runtime),
            reader: Box::new(started.reader),
            startup_messages: started.startup_messages,
        })
    }
    fn supports_native_resume(&self) -> bool {
        claude_adapter::supports_native_resume()
    }
    fn normalize(&self, value: &Value) -> Vec<agent::NormalizedEvent> {
        let session_key = value
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or("default")
            .to_owned();
        let mut streams = self.streams.lock().unwrap();
        let state = streams.entry(session_key).or_default();
        agent::normalize_claude_message_with_state(value, state)
    }
    fn forget_session(&self, provider_session_id: &str) {
        if provider_session_id == "default" {
            return;
        }
        self.streams.lock().unwrap().remove(provider_session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fake;
    impl HarnessAdapter for Fake {
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn descriptor(&self) -> AdapterDescriptor {
            AdapterDescriptor {
                sandbox_modes: crate::model::SandboxMode::ALL.to_vec(),
                id: "fake".into(),
                label: "Fake".into(),
                available: true,
                auth_state: crate::model::AuthState::Unknown,
                version: Some("1".into()),
                capabilities: vec!["messages".into()],
                unavailable_reason: None,
                models: vec![],
                default_model: None,
            }
        }
        fn start(&self, _request: StartRequest<'_>) -> Result<StartedAdapter, BridgeError> {
            Err(BridgeError::Invalid("not launched in registry test".into()))
        }
        fn resume(&self, _request: ResumeRequest<'_>) -> Result<StartedAdapter, BridgeError> {
            Err(BridgeError::Invalid("not resumed in registry test".into()))
        }
        fn supports_native_resume(&self) -> bool {
            false
        }
        fn normalize(&self, _value: &Value) -> Vec<agent::NormalizedEvent> {
            vec![]
        }
    }
    #[test]
    fn rejects_duplicate_ids() {
        let mut registry = AdapterRegistry {
            adapters: HashMap::new(),
        };
        registry.register(Box::new(Fake)).unwrap();
        let error = registry.register(Box::new(Fake)).unwrap_err();
        assert!(error.to_string().contains("Duplicate adapter id"));
    }
    #[test]
    fn capabilities_are_discovered_through_registry() {
        let mut registry = AdapterRegistry {
            adapters: HashMap::new(),
        };
        registry.register(Box::new(Fake)).unwrap();
        assert_eq!(registry.descriptors()[0].capabilities, vec!["messages"]);
    }

    /// The descriptor is the router's only source of truth about what a harness
    /// can start. OpenCode's read-only launch guard fails closed, so the
    /// descriptor must not advertise a mode the adapter always rejects.
    #[test]
    fn opencode_never_advertises_the_read_only_sandbox_it_refuses_to_start() {
        let registry = AdapterRegistry::built_in().unwrap();
        let opencode = registry
            .descriptors()
            .into_iter()
            .find(|descriptor| descriptor.id == "opencode")
            .expect("opencode adapter is registered");
        assert!(!opencode.supports_sandbox(SandboxMode::ReadOnly));
        assert!(opencode.supports_sandbox(SandboxMode::WorkspaceWrite));
        assert!(opencode.supports_sandbox(SandboxMode::DangerFullAccess));
        for descriptor in registry.descriptors() {
            assert!(
                !descriptor.sandbox_modes.is_empty(),
                "{} must declare its sandbox modes",
                descriptor.id
            );
        }
        for other in registry
            .descriptors()
            .into_iter()
            .filter(|descriptor| descriptor.id != "opencode")
        {
            assert!(
                other.supports_sandbox(SandboxMode::ReadOnly),
                "{} runs read-only workers",
                other.id
            );
        }
    }

    /// Install status and auth status are reported through separate fields:
    /// an unresolved binary must never coerce the credential probe into
    /// `SignedOut`, since the two can genuinely disagree (a user can sign in
    /// once and later uninstall the CLI).
    #[test]
    fn missing_binary_is_not_reported_as_signed_out() {
        for descriptor in AdapterRegistry::built_in().unwrap().descriptors() {
            if !descriptor.available {
                assert!(
                    descriptor.unavailable_reason.is_some(),
                    "{} is unavailable but names no reason",
                    descriptor.id
                );
            }
        }
        let unavailable_but_signed_in = AdapterDescriptor {
            id: "codex".into(),
            label: "Codex".into(),
            available: false,
            auth_state: crate::model::AuthState::SignedIn,
            version: None,
            capabilities: vec![],
            sandbox_modes: vec![],
            unavailable_reason: Some("Codex binary is not installed".into()),
            models: vec![],
            default_model: None,
        };
        assert!(!unavailable_but_signed_in.available);
        assert_eq!(unavailable_but_signed_in.auth_state, crate::model::AuthState::SignedIn);
    }

    #[test]
    fn every_advertised_model_has_one_tier_and_each_populated_tier_has_one_default() {
        let registry = AdapterRegistry::built_in().unwrap();
        for descriptor in registry.descriptors() {
            if !descriptor.available {
                continue;
            }
            assert!(!descriptor.models.is_empty());
            for tier in [
                CapabilityTier::Fast,
                CapabilityTier::Standard,
                CapabilityTier::Strong,
            ] {
                let models = descriptor
                    .models
                    .iter()
                    .filter(|model| model.tier == tier)
                    .collect::<Vec<_>>();
                if models.is_empty() {
                    continue;
                }
                assert_eq!(
                    models.iter().filter(|model| model.default_for_tier).count(),
                    1,
                    "{} must have exactly one {} default",
                    descriptor.id,
                    tier.as_str()
                );
            }
        }
    }

    #[test]
    fn tier_resolution_is_deterministic_and_falls_back_safely() {
        let registry = AdapterRegistry::built_in().unwrap();
        let default = registry
            .resolve_model("claude", CapabilityTier::Strong, None)
            .unwrap();
        assert_eq!(default.actual_model, "fable");
        assert!(default.warning.is_none());

        let known = registry
            .resolve_model("claude", CapabilityTier::Strong, Some("opus"))
            .unwrap();
        assert_eq!(known.actual_model, "opus");
        assert!(known.warning.is_none());

        for hint in ["not-installed", "haiku"] {
            let fallback = registry
                .resolve_model("claude", CapabilityTier::Strong, Some(hint))
                .unwrap();
            assert_eq!(fallback.actual_model, "fable");
            assert!(fallback
                .warning
                .as_deref()
                .is_some_and(|text| text.contains(hint)));
        }
    }

    #[cfg(unix)]
    #[test]
    fn stderr_tail_captures_a_dying_process_last_words() {
        let mut child = Command::new("/bin/sh")
            .args([
                "-c",
                "echo boot >&2; echo 'API error: connection refused' >&2; exit 7",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let tail = StderrTail::capture(&mut child);
        child.wait().unwrap();
        // The capture thread races the wait; poll briefly for the tail.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while tail.snapshot().is_none() && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let context = process_failure_context(&mut child, &tail).expect("context after exit");
        assert!(context.contains("exit status: 7"), "{context}");
        assert!(
            context.contains("API error: connection refused"),
            "{context}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn stderr_tail_is_bounded_to_the_last_lines() {
        let mut child = Command::new("/bin/sh")
            .args([
                "-c",
                "i=0; while [ $i -lt 100 ]; do echo line-$i >&2; i=$((i+1)); done",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let tail = StderrTail::capture(&mut child);
        child.wait().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while tail
            .snapshot()
            .is_none_or(|snapshot| !snapshot.contains("line-99"))
            && std::time::Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(10));
        }
        let snapshot = tail.snapshot().unwrap();
        assert!(snapshot.contains("line-99"));
        assert!(
            !snapshot.contains("line-69\n"),
            "older lines must be evicted"
        );
        assert_eq!(snapshot.lines().count(), STDERR_TAIL_LINES);
    }

    #[test]
    fn a_running_process_with_silent_stderr_has_no_failure_context() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "sleep 5"])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let tail = StderrTail::capture(&mut child);
        assert!(process_failure_context(&mut child, &tail).is_none());
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn forget_session_drops_stream_state_but_never_the_default_key() {
        let adapter = OpenCodeAdapter {
            streams: Mutex::new(HashMap::new()),
            settings: RwLock::new(Default::default()),
            catalog: Arc::new(RwLock::new(None)),
            catalog_error: Arc::new(RwLock::new(None)),
        };
        let with_session = serde_json::json!({
            "type": "message.updated",
            "properties": {"sessionID": "ses_1", "info": {"id": "m1", "role": "assistant"}}
        });
        let without_session = serde_json::json!({
            "type": "message.updated",
            "properties": {"info": {"id": "m2", "role": "assistant"}}
        });
        let _ = adapter.normalize(&with_session);
        let _ = adapter.normalize(&without_session);
        assert!(adapter.streams.lock().unwrap().contains_key("ses_1"));
        adapter.forget_session("ses_1");
        adapter.forget_session("default");
        let streams = adapter.streams.lock().unwrap();
        assert!(!streams.contains_key("ses_1"), "the ended session is dropped");
        assert!(
            streams.contains_key("default"),
            "the shared fallback entry survives per-session teardown"
        );
    }

    /// The wrapped child — and anything it forked into the group — must die
    /// when the supervisor is SIGKILLed, the path where no destructor, drain,
    /// or boot recovery can help; and everything must stay up while the
    /// supervisor lives.
    #[cfg(unix)]
    #[test]
    fn watchdog_reaps_child_and_group_mates_after_supervisor_sigkill() {
        use std::time::Instant;
        let stamp = std::process::id() % 1000;
        let mate_marker = format!("300.1{stamp:03}");
        let child_marker = format!("300.2{stamp:03}");
        // Exact-command patterns so neither the shells nor the intermediate
        // supervisor (whose argv carries the markers) satisfy the probes.
        let probe = |marker: &str| {
            let pattern = format!("^/bin/sleep {marker}$");
            Command::new("pgrep")
                .args(["-f", &pattern])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        };
        // The wrapped command forks a group-mate, then execs into the pid the
        // watchdog tracks — killing only that pid would leave the mate as a
        // PID-1 orphan. `set -m` gives the watchdog its own process group, as
        // configure_process_group does in production.
        let mut intermediate = Command::new("/bin/sh");
        intermediate
            .env("BRIDGE_WATCHDOG_UNDER_TEST", PARENT_WATCHDOG_SCRIPT)
            .env(
                "BRIDGE_WATCHDOG_INNER",
                format!("/bin/sleep {mate_marker} & exec /bin/sleep {child_marker}"),
            )
            .args([
                "-c",
                "set -m; /bin/sh -c 'eval \"$BRIDGE_WATCHDOG_UNDER_TEST\"' bridge-watchdog /bin/sh -c \"$BRIDGE_WATCHDOG_INNER\" & sleep 600",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        configure_process_group(&mut intermediate);
        let mut supervisor = intermediate.spawn().expect("intermediate supervisor spawns");

        let deadline = Instant::now() + Duration::from_secs(8);
        while !(probe(&mate_marker) && probe(&child_marker)) {
            assert!(
                Instant::now() < deadline,
                "the wrapped child and its group-mate never started"
            );
            thread::sleep(Duration::from_millis(100));
        }

        // Longer than a watchdog poll interval: a false trigger would have
        // reaped by now.
        thread::sleep(Duration::from_millis(2_500));
        assert!(
            probe(&mate_marker) && probe(&child_marker),
            "the watchdog must not reap while the supervisor lives"
        );

        let _ = Command::new("kill")
            .args(["-KILL", &supervisor.id().to_string()])
            .status();
        let _ = supervisor.wait();

        let deadline = Instant::now() + Duration::from_secs(12);
        while probe(&mate_marker) || probe(&child_marker) {
            assert!(
                Instant::now() < deadline,
                "the watchdog must reap the whole group once the supervisor dies"
            );
            thread::sleep(Duration::from_millis(200));
        }

        // The intermediate's own `sleep 600` shares its group; sweep it so
        // the test leaves nothing behind.
        let _ = terminate_process_group(supervisor.id());
    }
}

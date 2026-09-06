use crate::{
    agent,
    briefing_policy::BriefingRuntimePolicy,
    claude_adapter, codex_adapter, cursor_adapter,
    delegation::WriteMode,
    grok_adapter,
    model::{AdapterDescriptor, CapabilityTier, SandboxMode},
    model_catalog::{self, CatalogCandidate},
    opencode_adapter,
    worker_sandbox::ReadOnlySandbox,
    BridgeError,
};
use serde_json::Value;
use std::{
    any::Any,
    collections::HashMap,
    ffi::OsStr,
    io::BufRead,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex, RwLock},
    thread,
    time::Duration,
};

/// Trusted, application-owned context for one turn: Bridge's own words, never
/// folded into the visible user message.
///
/// Two named things rather than one blob, because they are owed for different
/// reasons and a provider that can name its context entries must not label one
/// as the other. `session` is the launch's session-context frame
/// (`session_context.rs`) — capabilities and memory, delivered in the
/// conversation tail so the system prompt stays byte-stable across restarts.
/// `credentials` is the per-turn capability contract, present only when the
/// visible text carries a `[secret:]` marker registered to this session.
#[derive(Debug, Clone, Copy, Default)]
pub struct TurnContext<'a> {
    pub session: Option<&'a str>,
    pub credentials: Option<&'a str>,
}

/// One present context entry. `name` is the wire key for providers that carry
/// named context entries (Codex's `additionalContext`); providers whose only
/// channel is the message body use `value` alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnContextEntry<'a> {
    pub name: &'static str,
    pub value: &'a str,
}

impl<'a> TurnContext<'a> {
    /// The entries actually present, in delivery order: the session frame
    /// first, because it is the standing contract the per-turn note refines.
    pub fn entries(&self) -> impl Iterator<Item = TurnContextEntry<'a>> {
        [
            ("bridge.session", self.session),
            ("bridge.credentials", self.credentials),
        ]
        .into_iter()
        .filter_map(|(name, value)| {
            let value = value.map(str::trim).filter(|value| !value.is_empty())?;
            Some(TurnContextEntry { name, value })
        })
    }

    pub fn is_empty(&self) -> bool {
        self.entries().next().is_none()
    }
}

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
        _context: TurnContext<'_>,
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
        _context: TurnContext<'_>,
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
    /// Resolve a permission with the exact provider option advertised on the
    /// request. Providers without option ids use the decision vocabulary.
    fn respond_with_option(
        &self,
        request_id: Value,
        decision: &str,
        _option_id: Option<&str>,
    ) -> Result<(), BridgeError> {
        self.respond(request_id, decision)
    }
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
//
// `/bin/kill` is invoked by full path rather than as a bare `kill`: dash's
// builtin (the `/bin/sh` on Debian/Ubuntu, unlike bash on macOS) rejects
// `-- -$$` with "Illegal number: -" and silently no-ops behind the
// `2>/dev/null` redirect, so the group was never actually signaled on Linux.
#[cfg(unix)]
const PARENT_WATCHDOG_SCRIPT: &str = r#"cmd="$1"; shift
# POSIX shells may attach /dev/null to an asynchronous command's stdin when
# job control is unavailable. Save the real pipe before spawning: dash applies
# that default before <&0, so duplicating fd 0 in the child just keeps /dev/null.
# Close the extra descriptor in both processes after wiring the child's stdin.
exec 3<&0
"$cmd" "$@" <&3 3<&- &
child=$!
exec 3<&-
trap 'trap "" TERM INT; /bin/kill -TERM -- -$$ 2>/dev/null' TERM INT
while kill -0 "$child" 2>/dev/null; do
  ppid=$(ps -o ppid= -p $$ 2>/dev/null | tr -d ' ')
  if [ -z "$ppid" ] || [ "$ppid" -le 1 ]; then
    trap '' TERM
    /bin/kill -TERM -- -$$ 2>/dev/null
    sleep 2
    /bin/kill -KILL -- -$$ 2>/dev/null
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
pub fn supervised_command<I, S>(executable: &Path, args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
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
pub fn supervised_command<I, S>(executable: &Path, args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(executable);
    command.args(args);
    command
}

/// The executable and argument vector for an ACP child protected by the same
/// parent-death watchdog as Bridge's `std::process::Command` children.
///
/// `agent-client-protocol` owns ACP spawning and accepts command parts rather
/// than a prepared [`Command`], so this adapts the shared watchdog to that
/// boundary. ACP arguments are already UTF-8 by the protocol crate contract.
#[cfg(unix)]
pub fn supervised_acp_parts(executable: &Path, args: &[String]) -> (PathBuf, Vec<String>) {
    if std::env::var_os(PARENT_WATCHDOG_DISABLE_ENV).is_some() {
        return (executable.to_path_buf(), args.to_vec());
    }
    let mut wrapped = vec![
        "-c".to_owned(),
        PARENT_WATCHDOG_SCRIPT.to_owned(),
        "bridge-watchdog".to_owned(),
        executable.to_string_lossy().into_owned(),
    ];
    wrapped.extend_from_slice(args);
    (PathBuf::from("/bin/sh"), wrapped)
}

#[cfg(not(unix))]
pub fn supervised_acp_parts(executable: &Path, args: &[String]) -> (PathBuf, Vec<String>) {
    (executable.to_path_buf(), args.to_vec())
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
    // The "--" is load-bearing on Linux: procps kill otherwise treats the
    // negative pid as another option token and exits 0 without signalling
    // anything. macOS's kill happens to tolerate the bare form.
    let signal = |value: &str| {
        Command::new("kill")
            .args([value, "--", &target])
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
    /// Fork `provider_session_id` into a NEW thread instead of resuming it in
    /// place. Codex-only (`thread/fork`); other adapters ignore it — which is
    /// why the native-fork restoration plan refuses adapters without native
    /// fork support before ever building this request.
    pub fork: bool,
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
            .field("fork", &self.fork)
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
    /// Whether [`ResumeRequest::fork`] can fork a stored provider thread into
    /// a new one. Defaults to false: only harnesses with an explicit fork verb
    /// (Codex `thread/fork`) opt in.
    fn supports_native_fork(&self) -> bool {
        false
    }
    fn normalize(&self, value: &Value) -> Vec<agent::NormalizedEvent>;
    /// Drop any normalization state kept for `provider_session_id`. Called
    /// when the session's runtime is gone; adapters without per-session state
    /// ignore it.
    fn forget_session(&self, _provider_session_id: &str) {}
    /// Look again at whatever decides this harness's availability.
    ///
    /// A default no-op, because most adapters read availability fresh on every
    /// descriptor and have nothing recorded to go stale. An adapter that proves
    /// availability with a handshake and records the answer overrides this to
    /// take the probe again — off-thread, announcing through the discovery
    /// notify when it lands — because the recorded answer can be changed by
    /// things Bridge does not observe, a sign-in above all.
    fn refresh_availability(&self) {}
}

pub struct AdapterRegistry {
    adapters: HashMap<String, Box<dyn HarnessAdapter>>,
}

/// One compatibility contract for settings, routing, and adapter startup.
/// Roles map to the authority they require; descriptors state what the runtime
/// can enforce. Orchestrators additionally require the briefing boundary.
pub fn descriptor_supports_agent_role(descriptor: &AdapterDescriptor, role: &str) -> bool {
    match role {
        "orchestrator" => {
            descriptor
                .capabilities
                .iter()
                .any(|value| value == "briefings")
                && descriptor.supports_sandbox(SandboxMode::WorkspaceWrite)
        }
        "implementation" => descriptor.supports_sandbox(SandboxMode::WorkspaceWrite),
        "research" | "verification" | "planning" | "documentation" => {
            descriptor.supports_sandbox(SandboxMode::ReadOnly)
        }
        _ => false,
    }
}

fn supported_model_effort<'a>(descriptor: &AdapterDescriptor, model: Option<&str>, effort: Option<&'a str>) -> Option<&'a str> {
    let selected = descriptor.models.iter().find(|option|
        Some(option.id.as_str()) == model.or(descriptor.default_model.as_deref()));
    effort.filter(|value| selected.is_some_and(|model|
        model.supported_effort_levels.iter().any(|level| level == value)))
}

fn validate_start_compatibility(
    descriptor: &AdapterDescriptor,
    request: &StartRequest<'_>,
) -> Result<(), BridgeError> {
    if request.briefing.is_some()
        && !descriptor
            .capabilities
            .iter()
            .any(|value| value == "briefings")
    {
        return Err(BridgeError::Invalid(format!(
            "{} cannot run orchestrator briefings",
            descriptor.label
        )));
    }
    let sandbox = if request.read_only_sandbox.is_some()
        || matches!(request.write_mode, Some(WriteMode::ReadOnly))
    {
        Some(SandboxMode::ReadOnly)
    } else {
        match request.write_mode {
            Some(WriteMode::Full) => Some(SandboxMode::DangerFullAccess),
            Some(WriteMode::Shared | WriteMode::Isolated) => Some(SandboxMode::WorkspaceWrite),
            None => None,
            Some(WriteMode::ReadOnly) => unreachable!(),
        }
    };
    if sandbox.is_some_and(|mode| !descriptor.supports_sandbox(mode)) {
        return Err(BridgeError::Invalid(format!(
            "{} cannot enforce the requested worker sandbox",
            descriptor.label
        )));
    }
    Ok(())
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

    /// `on_discovered` fires once a background adapter discovery finishes
    /// (successfully or not), so the host can tell the frontend to re-read
    /// adapter availability. Every adapter that discovers itself off-thread
    /// gets one: a probe that lands silently leaves its harness greyed out in
    /// the picker until some unrelated read happens to refresh health.
    pub fn built_in_with_opencode_notify(
        opencode_settings: opencode_adapter::OpenCodeSettings,
        on_discovered: Option<Box<dyn Fn() + Send + Sync>>,
    ) -> Result<Self, BridgeError> {
        Self::built_in_with_opencode_notify_and_cache(opencode_settings, on_discovered, None)
    }

    pub fn built_in_with_opencode_notify_and_cache(
        opencode_settings: opencode_adapter::OpenCodeSettings,
        on_discovered: Option<Box<dyn Fn() + Send + Sync>>,
        opencode_cache_path: Option<PathBuf>,
    ) -> Result<Self, BridgeError> {
        let mut registry = Self {
            adapters: HashMap::new(),
        };
        let on_discovered: Option<Arc<dyn Fn() + Send + Sync>> = on_discovered.map(Arc::from);
        registry.register(Box::new(CodexAdapter::new(on_discovered.clone())))?;
        registry.register(Box::new(ClaudeAdapter::new(on_discovered.clone())))?;
        let notify = |shared: &Option<Arc<dyn Fn() + Send + Sync>>| {
            shared
                .clone()
                .map(|shared| Box::new(move || shared()) as Box<dyn FnOnce() + Send>)
        };
        registry.register(Box::new(OpenCodeAdapter::new(
            opencode_settings,
            notify(&on_discovered),
            opencode_cache_path,
        )))?;
        // Cursor discovers itself the same way, and for a stronger reason: its
        // protocol support cannot be read off the filesystem and has to be
        // proved with a handshake, which is not something application setup can
        // wait on. It keeps the shared notify rather than a one-shot: its probe
        // is retaken after a sign-in, and each retake announces itself too.
        registry.register(Box::new(cursor_adapter::CursorAdapter::new(
            on_discovered.clone(),
        )))?;
        registry.register(Box::new(grok_adapter::GrokAdapter::new(on_discovered)))?;
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

    /// Ask one adapter to re-establish its availability.
    ///
    /// A no-op for an unknown id: the caller names whichever provider just
    /// finished a sign-in, and not every provider has a structured adapter.
    pub fn refresh_availability(&self, id: &str) {
        if let Some(adapter) = self.adapters.get(id) {
            adapter.refresh_availability();
        }
    }

    pub fn refresh_model_catalogs(&self) {
        for adapter in self.adapters.values() {
            adapter.refresh_availability();
        }
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
        mut request: StartRequest<'_>,
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
        validate_start_compatibility(&descriptor, &request)?;
        request.effort = supported_model_effort(&descriptor, request.model, request.effort);
        adapter.start(request)
    }

    pub fn resume(
        &self,
        id: &str,
        mut request: ResumeRequest<'_>,
    ) -> Result<StartedAdapter, BridgeError> {
        let adapter = self.adapters.get(id).ok_or_else(|| {
            BridgeError::Invalid(format!("No structured adapter is registered for {id}"))
        })?;
        if request.fork && !adapter.supports_native_fork() {
            return Err(BridgeError::Invalid(format!(
                "Adapter {id} does not support native thread forks"
            )));
        }
        if !adapter.supports_native_resume() {
            return Err(BridgeError::Invalid(format!(
                "Adapter {id} does not support native resume"
            )));
        }
        let descriptor = adapter.descriptor();
        request.effort = supported_model_effort(&descriptor, request.model, request.effort);
        adapter.resume(request)
    }

    pub fn supports_native_resume(&self, id: &str) -> bool {
        self.adapters
            .get(id)
            .is_some_and(|adapter| adapter.supports_native_resume())
    }

    pub fn supports_native_fork(&self, id: &str) -> bool {
        self.adapters
            .get(id)
            .is_some_and(|adapter| adapter.supports_native_fork())
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
        let selectable = descriptor
            .models
            .iter()
            .filter(|model| model.available && model.compatible)
            .collect::<Vec<_>>();
        let unranked = selectable
            .first()
            .is_some_and(|first| selectable.iter().all(|model| model.tier == first.tier));
        let tier_default = selectable
            .iter()
            .copied()
            .find(|model| model.tier == tier && model.default_for_tier)
            .or_else(|| selectable.iter().copied().find(|model| model.tier == tier))
            .or_else(|| {
                unranked
                    .then(|| {
                        selectable
                            .iter()
                            .copied()
                            .find(|model| model.default_for_tier)
                    })
                    .flatten()
            })
            .or_else(|| unranked.then(|| selectable.first().copied()).flatten())
            .ok_or_else(|| {
                BridgeError::Invalid(format!(
                    "Adapter {id} does not advertise a {} capability model",
                    tier.as_str()
                ))
            })?;
        let hinted = model_hint.and_then(|hint| {
            selectable
                .iter()
                .copied()
                .find(|model| model.id.eq_ignore_ascii_case(hint.trim()))
        });
        let selected = hinted
            .filter(|model| unranked || model.tier == tier)
            .unwrap_or(tier_default);
        let warning = model_hint.and_then(|hint| {
            (hinted.is_none() || hinted.is_some_and(|model| !unranked && model.tier != tier)).then(
                || {
                    format!(
                        "Model hint {hint:?} is unknown or outside tier {}; using {}",
                        tier.as_str(),
                        tier_default.id
                    )
                },
            )
        });
        Ok(ModelResolution {
            requested_tier: tier,
            actual_model: selected.id.clone(),
            warning,
        })
    }

    /// Resolve a model a session pinned itself to, preserving the pinned model's
    /// own tier when it is still selectable. A pin that has dropped out of the
    /// live catalogue (a renamed or retired id, e.g. an old `fable-5-1`) falls
    /// back to the Standard tier default with a warning rather than failing the
    /// session start with a raw provider "unknown model" error.
    pub fn resolve_pinned_model(
        &self,
        id: &str,
        model: &str,
    ) -> Result<ModelResolution, BridgeError> {
        let descriptor = self
            .adapters
            .get(id)
            .ok_or_else(|| {
                BridgeError::Invalid(format!("No structured adapter is registered for {id}"))
            })?
            .descriptor();
        let tier = descriptor
            .models
            .iter()
            .find(|option| {
                option.id.eq_ignore_ascii_case(model.trim())
                    && option.available
                    && option.compatible
            })
            .map(|option| option.tier)
            .unwrap_or(CapabilityTier::Standard);
        self.resolve_model(id, tier, Some(model))
    }
}

struct OpenCodeAdapter {
    streams: Mutex<HashMap<String, agent::OpenCodeStreamState>>,
    settings: RwLock<opencode_adapter::OpenCodeSettings>,
    catalog: Arc<RwLock<Option<opencode_adapter::OpenCodeCatalog>>>,
    catalog_error: Arc<RwLock<Option<String>>>,
    model_catalog: Arc<RwLock<model_catalog::ResolvedCatalog>>,
    cache_path: Option<PathBuf>,
}
impl OpenCodeAdapter {
    fn new(
        settings: opencode_adapter::OpenCodeSettings,
        on_discovered: Option<Box<dyn FnOnce() + Send>>,
        cache_path: Option<PathBuf>,
    ) -> Self {
        let initial_models = model_catalog::resolve(
            "opencode",
            Err("OpenCode discovery has not completed".into()),
            &[],
            cache_path.as_deref(),
            chrono::Utc::now(),
        );
        let adapter = Self {
            streams: Mutex::new(HashMap::new()),
            settings: RwLock::new(settings.clone()),
            catalog: Arc::new(RwLock::new(None)),
            catalog_error: Arc::new(RwLock::new(None)),
            model_catalog: Arc::new(RwLock::new(initial_models)),
            cache_path: cache_path.clone(),
        };
        // Discovery spawns an OpenCode server and can take tens of seconds, and
        // new() runs during app setup — do the initial catalog load off-thread.
        let catalog = adapter.catalog.clone();
        let catalog_error = adapter.catalog_error.clone();
        let model_catalog = adapter.model_catalog.clone();
        let directory = std::env::current_dir()
            .ok()
            .and_then(|path| path.to_str().map(str::to_owned))
            .unwrap_or_else(|| ".".into());
        let _ = std::thread::Builder::new()
            .name("opencode-discover".into())
            .spawn(move || {
                match opencode_adapter::discover(&settings, &directory) {
                    Ok(result) => {
                        let raw_options =
                            opencode_adapter::model_options(&result, &settings.visible_models);
                        let candidates = raw_options
                            .into_iter()
                            .map(|model| CatalogCandidate {
                                id: model.id,
                                label: model.label,
                                tier: model.tier,
                                available: model.available,
                                compatible: model.compatible,
                                lifecycle: model.lifecycle,
                                supported_effort_levels: model.supported_effort_levels,
                                promotion_priority: i64::from(model.default_for_tier),
                            })
                            .collect();
                        *model_catalog.write().unwrap() = model_catalog::resolve(
                            "opencode",
                            Ok(candidates),
                            &[],
                            cache_path.as_deref(),
                            chrono::Utc::now(),
                        );
                        *catalog.write().unwrap() = Some(result);
                        *catalog_error.write().unwrap() = None;
                    }
                    Err(error) => {
                        let message = error.to_string();
                        let failed = model_catalog::resolve(
                            "opencode",
                            Err(message.clone()),
                            &[],
                            cache_path.as_deref(),
                            chrono::Utc::now(),
                        );
                        if !failed.models.is_empty() {
                            *model_catalog.write().unwrap() = failed;
                        } else {
                            let mut current = model_catalog.write().unwrap();
                            current.diagnostics.stale = true;
                            current.diagnostics.last_error = Some(message.clone());
                        }
                        *catalog_error.write().unwrap() = Some(message);
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
                let raw_options =
                    opencode_adapter::model_options(&catalog, &settings.visible_models);
                let candidates = raw_options
                    .into_iter()
                    .map(|model| CatalogCandidate {
                        id: model.id,
                        label: model.label,
                        tier: model.tier,
                        available: model.available,
                        compatible: model.compatible,
                        lifecycle: model.lifecycle,
                        supported_effort_levels: model.supported_effort_levels,
                        promotion_priority: i64::from(model.default_for_tier),
                    })
                    .collect();
                *self.model_catalog.write().unwrap() = model_catalog::resolve(
                    "opencode",
                    Ok(candidates),
                    &[],
                    self.cache_path.as_deref(),
                    chrono::Utc::now(),
                );
                *self.catalog.write().unwrap() = Some(catalog.clone());
                *self.catalog_error.write().unwrap() = None;
                Ok(catalog)
            }
            Err(error) => {
                // Keep the last known-good catalog so a transient discovery
                // failure does not degrade a working setup; the error is
                // surfaced alongside it.
                let message = error.to_string();
                let failed = model_catalog::resolve(
                    "opencode",
                    Err(message.clone()),
                    &[],
                    self.cache_path.as_deref(),
                    chrono::Utc::now(),
                );
                if !failed.models.is_empty() {
                    *self.model_catalog.write().unwrap() = failed;
                } else {
                    let mut current = self.model_catalog.write().unwrap();
                    current.diagnostics.stale = true;
                    current.diagnostics.last_error = Some(message.clone());
                }
                *self.catalog_error.write().unwrap() = Some(message);
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
            .any(|option| option.id == model && option.available && option.compatible);
        selectable.then_some(()).ok_or_else(|| {
            BridgeError::Invalid(format!(
                "OpenCode model {model:?} is not exposed by a connected provider or is hidden"
            ))
        })
    }

    fn replace_catalog(&self, catalog: opencode_adapter::OpenCodeCatalog) {
        let raw_options = opencode_adapter::model_options(
            &catalog,
            &self.settings.read().unwrap().visible_models,
        );
        let candidates = raw_options
            .into_iter()
            .map(|model| CatalogCandidate {
                id: model.id,
                label: model.label,
                tier: model.tier,
                available: model.available,
                compatible: model.compatible,
                lifecycle: model.lifecycle,
                supported_effort_levels: model.supported_effort_levels,
                promotion_priority: i64::from(model.default_for_tier),
            })
            .collect();
        *self.model_catalog.write().unwrap() = model_catalog::resolve(
            "opencode",
            Ok(candidates),
            &[],
            self.cache_path.as_deref(),
            chrono::Utc::now(),
        );
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
        let resolved_catalog = self.model_catalog.read().unwrap().clone();
        let models = resolved_catalog.models;
        let default_model = models
            .iter()
            .find(|model| model.tier == CapabilityTier::Standard && model.default_for_tier)
            .or_else(|| models.iter().find(|model| model.default_for_tier))
            .map(|model| model.id.clone());
        let runtime_available = crate::managed_runtime::managed_entrypoint("opencode").is_some()
            || crate::binary::resolve("opencode").is_some();
        let available = runtime_available
            && models
                .iter()
                .any(|model| model.available && model.compatible);
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
                "briefings",
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
            model_catalog: resolved_catalog.diagnostics,
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

fn inferred_tier(id: &str, label: &str) -> CapabilityTier {
    let name = format!("{id} {label}").to_ascii_lowercase();
    if ["haiku", "mini", "nano", "luna", "fast"]
        .iter()
        .any(|part| name.contains(part))
    {
        CapabilityTier::Fast
    } else if ["opus", "fable", "sol", "strong", "pro", "max"]
        .iter()
        .any(|part| name.contains(part))
    {
        CapabilityTier::Strong
    } else {
        CapabilityTier::Standard
    }
}

/// One model a provider reports at discovery time, before Bridge's promotion
/// policy runs. Adapters supply facts only; `is_default` and the effort levels
/// come straight off the provider's own catalogue row.
#[derive(Debug, Clone)]
pub struct DiscoveredModel {
    pub id: String,
    pub label: String,
    /// The provider marks this as its own default for the (inferred) tier.
    pub is_default: bool,
    /// Reasoning effort levels the provider says this model accepts.
    pub supported_effort_levels: Vec<String>,
}

/// Priority handed to a discovered model the provider marks as its default. Set
/// far above any curated priority so the provider's newest default wins its tier
/// promotion over Bridge's curated fallback.
const DISCOVERED_DEFAULT_PRIORITY: i64 = 1_000;

/// The id a descriptor advertises as its default: Bridge's promoted Standard
/// model, else any promoted tier default, else the provider's own fallback id.
/// Reflects a live catalogue whose promoted default outran the curated one,
/// rather than a hardcoded model that discovery has since superseded.
fn promoted_default_model(models: &[crate::model::ModelOption], fallback: &str) -> Option<String> {
    models
        .iter()
        .find(|model| {
            model.tier == CapabilityTier::Standard
                && model.default_for_tier
                && model.available
                && model.compatible
        })
        .or_else(|| {
            models
                .iter()
                .find(|model| model.default_for_tier && model.available && model.compatible)
        })
        .map(|model| model.id.clone())
        .or_else(|| Some(fallback.to_owned()))
}

fn runtime_candidates_with_fallbacks(
    models: Vec<DiscoveredModel>,
    fallbacks: &[CatalogCandidate],
) -> Vec<CatalogCandidate> {
    let candidates: Vec<CatalogCandidate> = models
        .into_iter()
        .enumerate()
        .map(|(index, model)| {
            let DiscoveredModel {
                id,
                label,
                is_default,
                supported_effort_levels,
            } = model;
            match fallbacks.iter().find(|fallback| fallback.id == id) {
                // Curated entries stay authoritative for tier; discovery
                // refreshes the display name and effort levels. A model the
                // provider now marks default outranks the curated priority so
                // the live default wins its tier, otherwise curation holds. This
                // keeps tier defaults deterministic across machines whatever
                // order a live provider lists its models in.
                Some(fallback) => {
                    let mut merged = fallback.clone();
                    merged.label = label;
                    merged.supported_effort_levels = supported_effort_levels;
                    if is_default {
                        merged.promotion_priority = DISCOVERED_DEFAULT_PRIORITY;
                    }
                    merged
                }
                None => {
                    let mut candidate = CatalogCandidate::stable(
                        id.clone(),
                        label.clone(),
                        inferred_tier(&id, &label),
                        // A provider default wins its tier; every other
                        // discovery stays strictly below curated priority so it
                        // never steals a tier default.
                        if is_default {
                            DISCOVERED_DEFAULT_PRIORITY
                        } else {
                            -1 - (index as i64)
                        },
                    );
                    candidate.supported_effort_levels = supported_effort_levels;
                    candidate
                }
            }
        })
        .collect();

    candidates
}

struct CodexAdapter {
    streams: Mutex<HashMap<String, agent::CodexStreamState>>,
    models: Arc<RwLock<model_catalog::ResolvedCatalog>>,
    notify: Option<Arc<dyn Fn() + Send + Sync>>,
    refreshing: Arc<AtomicBool>,
}

impl CodexAdapter {
    fn new(notify: Option<Arc<dyn Fn() + Send + Sync>>) -> Self {
        let fallback = codex_fallback_candidates();
        let models = Arc::new(RwLock::new(model_catalog::resolve(
            "codex",
            Err("Codex discovery has not completed".into()),
            &fallback,
            None,
            chrono::Utc::now(),
        )));
        let adapter = Self {
            streams: Mutex::new(HashMap::new()),
            models,
            notify,
            refreshing: Arc::new(AtomicBool::new(false)),
        };
        adapter.refresh_availability();
        adapter
    }
}
fn codex_fallback_candidates() -> Vec<CatalogCandidate> {
    vec![
        CatalogCandidate::stable("gpt-5.6-luna", "GPT Luna", CapabilityTier::Fast, 1),
        CatalogCandidate::stable("gpt-5.6-terra", "GPT Terra", CapabilityTier::Standard, 1),
        CatalogCandidate::stable("gpt-5.6-sol", "GPT Sol", CapabilityTier::Strong, 1),
        CatalogCandidate::stable(
            "gpt-5.3-codex",
            "GPT-5.3 Codex",
            CapabilityTier::Standard,
            0,
        ),
    ]
}

impl HarnessAdapter for CodexAdapter {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn descriptor(&self) -> AdapterDescriptor {
        let version = codex_adapter::binary_version();
        // One owned snapshot releases the read lock before building the
        // descriptor. Re-entering it can deadlock behind a pending refresh
        // writer while an earlier field's temporary guard is still alive.
        let catalog = self.models.read().unwrap().clone();
        let default_model = promoted_default_model(&catalog.models, "gpt-5.6-luna");
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
                "briefings",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            sandbox_modes: SandboxMode::ALL.to_vec(),
            unavailable_reason: codex_adapter::resolve_runtime()
                .is_none()
                .then(|| "Codex binary is not installed".into()),
            models: catalog.models,
            default_model,
            model_catalog: catalog.diagnostics,
        }
    }
    fn refresh_availability(&self) {
        if self.refreshing.swap(true, Ordering::AcqRel) { return; }
        let refreshing = self.refreshing.clone();
        let models = self.models.clone();
        let notify = self.notify.clone();
        thread::spawn(move || {
            let fallback = codex_fallback_candidates();
            let discovered = codex_adapter::discover_models()
                .map(|models| runtime_candidates_with_fallbacks(models, &fallback))
                .map_err(|error| error.to_string());
            let mut current = models.write().unwrap();
            if let Err(error) = &discovered {
                if current.diagnostics.source == crate::model::ModelCatalogSource::RuntimeApi {
                    current.diagnostics.stale = true;
                    current.diagnostics.last_error = Some(error.clone());
                    drop(current);
                    refreshing.store(false, Ordering::Release);
                    if let Some(notify) = notify { notify(); }
                    return;
                }
            }
            *current = model_catalog::resolve(
                "codex",
                discovered,
                &fallback,
                None,
                chrono::Utc::now(),
            );
            drop(current);
            refreshing.store(false, Ordering::Release);
            if let Some(notify) = notify {
                notify();
            }
        });
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
    fn supports_native_fork(&self) -> bool {
        codex_adapter::supports_native_fork()
    }
    fn normalize(&self, value: &Value) -> Vec<agent::NormalizedEvent> {
        if value.get("id").is_some() && value.get("method").is_some() {
            agent::normalize_codex_request(value).into_iter().collect()
        } else {
            // Standard Codex app-server stdio notifications carry threadId on
            // thread/turn boundaries, but omit session identity on mid-turn deltas.
            // Stream state therefore resolves the session key when present and
            // safely defaults to "default", with per-turn counter resets in
            // CodexStreamState preventing cross-turn reasoning collision.
            let session_key = value
                .pointer("/params/conversationId")
                .or_else(|| value.pointer("/params/threadId"))
                .or_else(|| value.pointer("/params/sessionID"))
                .and_then(Value::as_str)
                .unwrap_or("default")
                .to_owned();
            let mut streams = self.streams.lock().unwrap();
            let state = streams.entry(session_key).or_default();
            agent::normalize_codex_message_with_state(value, state)
        }
    }
    fn forget_session(&self, provider_session_id: &str) {
        if provider_session_id != "default" {
            self.streams.lock().unwrap().remove(provider_session_id);
        }
    }
}

struct ClaudeAdapter {
    streams: Mutex<HashMap<String, agent::ClaudeStreamState>>,
    models: Arc<RwLock<model_catalog::ResolvedCatalog>>,
    notify: Option<Arc<dyn Fn() + Send + Sync>>,
    refreshing: Arc<AtomicBool>,
}
impl ClaudeAdapter {
    fn new(notify: Option<Arc<dyn Fn() + Send + Sync>>) -> Self {
        let fallback = claude_fallback_candidates();
        let models = Arc::new(RwLock::new(model_catalog::resolve(
            "claude",
            Err("Claude discovery has not completed".into()),
            &fallback,
            None,
            chrono::Utc::now(),
        )));
        let adapter = Self {
            streams: Mutex::new(HashMap::new()),
            models,
            notify,
            refreshing: Arc::new(AtomicBool::new(false)),
        };
        adapter.refresh_availability();
        adapter
    }
}
fn claude_fallback_candidates() -> Vec<CatalogCandidate> {
    vec![
        CatalogCandidate::stable("haiku", "Claude Haiku", CapabilityTier::Fast, 1),
        CatalogCandidate::stable("sonnet", "Claude Sonnet", CapabilityTier::Standard, 1),
        CatalogCandidate::stable("opus", "Claude Opus", CapabilityTier::Strong, 1),
    ]
}
impl HarnessAdapter for ClaudeAdapter {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn descriptor(&self) -> AdapterDescriptor {
        let version = claude_adapter::binary_version();
        // Keep the model list, default, and diagnostics from the same read,
        // without recursively locking against a concurrent discovery writer.
        let catalog = self.models.read().unwrap().clone();
        let default_model = promoted_default_model(&catalog.models, claude_adapter::DEFAULT_MODEL);
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
                // Edit/Write/MultiEdit/NotebookEdit normalize to file_change.*
                // events with a synthesized diff, so Claude advertises the same
                // file_changes surface as Codex/OpenCode.
                "file_changes",
                "approvals",
                "usage",
                "interrupt",
                "briefings",
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
            models: catalog.models,
            default_model,
            model_catalog: catalog.diagnostics,
        }
    }
    fn refresh_availability(&self) {
        if self.refreshing.swap(true, Ordering::AcqRel) { return; }
        let refreshing = self.refreshing.clone();
        let models = self.models.clone();
        let notify = self.notify.clone();
        thread::spawn(move || {
            let fallback = claude_fallback_candidates();
            let discovered = claude_adapter::discover_models()
                .map(|models| runtime_candidates_with_fallbacks(models, &fallback))
                .map_err(|error| error.to_string());
            let mut current = models.write().unwrap();
            if let Err(error) = &discovered {
                if current.diagnostics.source == crate::model::ModelCatalogSource::RuntimeApi {
                    current.diagnostics.stale = true;
                    current.diagnostics.last_error = Some(error.clone());
                    drop(current);
                    refreshing.store(false, Ordering::Release);
                    if let Some(notify) = notify { notify(); }
                    return;
                }
            }
            *current = model_catalog::resolve(
                "claude",
                discovered,
                &fallback,
                None,
                chrono::Utc::now(),
            );
            drop(current);
            refreshing.store(false, Ordering::Release);
            if let Some(notify) = notify {
                notify();
            }
        });
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
    use crate::model::ModelCatalogDiagnostics;

    fn descriptor_completes_during_catalog_refresh(
        make_adapter: impl FnOnce(Arc<RwLock<model_catalog::ResolvedCatalog>>) -> Box<dyn HarnessAdapter>,
    ) {
        let candidates = (0..model_catalog::MAX_CATALOG_ENTRIES)
            .map(|index| CatalogCandidate::stable(
                format!("test-model-{index}"),
                format!("Test model {index}"),
                CapabilityTier::Standard,
                index as i64,
            ))
            .collect();
        let models = Arc::new(RwLock::new(model_catalog::resolve(
            "test", Ok(candidates), &[], None, chrono::Utc::now(),
        )));
        let adapter = make_adapter(models.clone());
        let stop = Arc::new(AtomicBool::new(false));
        let start = Arc::new(std::sync::Barrier::new(2));
        let writer = {
            let stop = stop.clone();
            let start = start.clone();
            thread::spawn(move || {
                start.wait();
                let mut generation = 0;
                while !stop.load(Ordering::Acquire) {
                    {
                        let mut current = models.write().unwrap();
                        let label = generation.to_string();
                        current.models[0].label = label.clone();
                        current.diagnostics.last_error = Some(label);
                    }
                    generation += 1;
                    thread::yield_now();
                }
            })
        };
        let (done, completed) = std::sync::mpsc::channel();
        let reader = thread::spawn(move || {
            start.wait();
            for _ in 0..64 {
                let descriptor = adapter.descriptor();
                assert_eq!(descriptor.models.len(), model_catalog::MAX_CATALOG_ENTRIES);
                assert!(descriptor.models.iter().any(|model| {
                    Some(&model.id) == descriptor.default_model.as_ref()
                }));
                if let Some(generation) = descriptor.model_catalog.last_error {
                    assert_eq!(descriptor.models[0].label, generation);
                }
            }
            done.send(()).unwrap();
        });
        // A recursive read can deadlock behind the pending refresh writer on
        // Linux. Bound the regression itself so it fails instead of hanging CI.
        let result = completed.recv_timeout(Duration::from_secs(30));
        stop.store(true, Ordering::Release);
        result.expect("descriptor reads must complete while the catalog is refreshed");
        reader.join().unwrap();
        writer.join().unwrap();
    }

    #[test]
    fn codex_descriptors_complete_during_catalog_refresh() {
        descriptor_completes_during_catalog_refresh(|models| Box::new(CodexAdapter {
            streams: Mutex::new(HashMap::new()),
            models,
            notify: None,
            refreshing: Arc::new(AtomicBool::new(false)),
        }));
    }

    #[test]
    fn claude_descriptors_complete_during_catalog_refresh() {
        descriptor_completes_during_catalog_refresh(|models| Box::new(ClaudeAdapter {
            streams: Mutex::new(HashMap::new()),
            models,
            notify: None,
            refreshing: Arc::new(AtomicBool::new(false)),
        }));
    }

    fn discovered(id: &str, label: &str) -> DiscoveredModel {
        DiscoveredModel {
            id: id.into(),
            label: label.into(),
            is_default: false,
            supported_effort_levels: Vec::new(),
        }
    }

    #[test]
    fn launch_effort_cannot_leak_from_a_different_model() {
        let mut descriptor = Fake.descriptor();
        let mut candidate = CatalogCandidate::stable("test-model", "Test", CapabilityTier::Standard, 1);
        candidate.supported_effort_levels = vec!["ultra".into()];
        descriptor.models = model_catalog::normalize(crate::model::ModelCatalogSource::RuntimeApi, [candidate]);
        let id = descriptor.models[0].id.as_str();
        assert_eq!(supported_model_effort(&descriptor, Some(id), Some("ultra")), Some("ultra"));
        assert_eq!(supported_model_effort(&descriptor, Some(id), Some("high")), None);
        assert_eq!(supported_model_effort(&descriptor, Some("retired"), Some("ultra")), None);
    }

    #[test]
    fn runtime_catalog_excludes_fallback_only_models() {
        let fallback = claude_fallback_candidates();
        let candidates = runtime_candidates_with_fallbacks(
            vec![discovered("claude-fable-5-1", "Claude Fable 5.1 Latest")],
            &fallback,
        );

        assert!(candidates.iter().any(|model| {
            model.id == "claude-fable-5-1" && model.label == "Claude Fable 5.1 Latest"
        }));
        assert_eq!(candidates.len(), 1);
    }

    #[test]
    fn canonical_duplicates_collapse_without_hiding_other_releases() {
        let candidates = runtime_candidates_with_fallbacks(vec![
            discovered("claude-opus-5", "Opus 5"),
            discovered("claude-opus-5", "Opus 5"),
            discovered("claude-opus-4-8", "Opus 4.8"),
        ], &claude_fallback_candidates());
        let models = model_catalog::normalize(crate::model::ModelCatalogSource::RuntimeApi, candidates);
        assert_eq!(models.len(), 2);
        assert!(models.iter().any(|model| model.id == "claude-opus-5"));
        assert!(models.iter().any(|model| model.id == "claude-opus-4-8"));
    }

    #[test]
    fn discovered_effort_levels_flow_onto_the_selectable_model() {
        let fallback = claude_fallback_candidates();
        let mut sonnet = discovered("sonnet", "Claude Sonnet");
        sonnet.supported_effort_levels = vec!["low".into(), "high".into(), "xhigh".into()];
        let haiku = discovered("haiku", "Claude Haiku");
        let resolved = model_catalog::normalize(
            crate::model::ModelCatalogSource::RuntimeApi,
            runtime_candidates_with_fallbacks(vec![sonnet, haiku], &fallback),
        );
        let sonnet = resolved.iter().find(|model| model.id == "sonnet").unwrap();
        assert_eq!(sonnet.supported_effort_levels, ["low", "high", "xhigh"]);
        // A model that reports no effort levels stays empty — the picker hides
        // the control rather than offering a fixed list it does not accept.
        let haiku = resolved.iter().find(|model| model.id == "haiku").unwrap();
        assert!(haiku.supported_effort_levels.is_empty());
    }

    #[test]
    fn discovered_models_never_steal_curated_tier_defaults() {
        let fallback = codex_fallback_candidates();
        let candidates = runtime_candidates_with_fallbacks(
            vec![
                discovered("gpt-6-new", "GPT-6 New"),
                discovered("gpt-5.6-terra", "GPT Terra Refreshed"),
            ],
            &fallback,
        );
        let resolved =
            model_catalog::normalize(crate::model::ModelCatalogSource::RuntimeApi, candidates);

        let standard_default = resolved
            .iter()
            .find(|model| model.tier == CapabilityTier::Standard && model.default_for_tier)
            .unwrap();
        // A discovery the provider does not mark default keeps curation
        // authoritative; discovery only refreshed the display name and added the
        // new release alongside it.
        assert_eq!(standard_default.id, "gpt-5.6-terra");
        assert_eq!(standard_default.label, "GPT Terra Refreshed");
        assert!(resolved
            .iter()
            .any(|model| model.id == "gpt-6-new" && !model.default_for_tier));
    }

    #[test]
    fn a_discovered_provider_default_becomes_the_tier_default() {
        let fallback = codex_fallback_candidates();
        // A brand-new model the provider now marks as its own default. It infers
        // to Standard and must outrank the curated Standard default.
        let mut astra = discovered("gpt-6-astra", "GPT Astra");
        astra.is_default = true;
        let candidates = runtime_candidates_with_fallbacks(
            vec![astra, discovered("gpt-5.6-terra", "GPT Terra")],
            &fallback,
        );
        let resolved =
            model_catalog::normalize(crate::model::ModelCatalogSource::RuntimeApi, candidates);
        let standard_default = resolved
            .iter()
            .find(|model| model.tier == CapabilityTier::Standard && model.default_for_tier)
            .unwrap();
        assert_eq!(standard_default.id, "gpt-6-astra");
        // The curated model is still selectable, just no longer the default.
        assert!(resolved
            .iter()
            .any(|model| model.id == "gpt-5.6-terra" && !model.default_for_tier));
    }

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
                model_catalog: ModelCatalogDiagnostics::curated(),
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
    fn refresh_reaches_the_named_adapter_and_ignores_everything_else() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static REFRESHED: AtomicUsize = AtomicUsize::new(0);
        struct Refreshing;
        impl HarnessAdapter for Refreshing {
            fn as_any(&self) -> &dyn Any {
                self
            }
            fn descriptor(&self) -> AdapterDescriptor {
                AdapterDescriptor {
                    sandbox_modes: crate::model::SandboxMode::ALL.to_vec(),
                    id: "refreshing".into(),
                    label: "Refreshing".into(),
                    available: true,
                    auth_state: crate::model::AuthState::Unknown,
                    version: Some("1".into()),
                    capabilities: vec!["messages".into()],
                    unavailable_reason: None,
                    models: vec![],
                    default_model: None,
                    model_catalog: ModelCatalogDiagnostics::curated(),
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
            fn refresh_availability(&self) {
                REFRESHED.fetch_add(1, Ordering::SeqCst);
            }
        }
        let mut registry = AdapterRegistry {
            adapters: HashMap::new(),
        };
        registry.register(Box::new(Refreshing)).unwrap();
        registry.register(Box::new(Fake)).unwrap();
        registry.refresh_availability("refreshing");
        assert_eq!(REFRESHED.load(Ordering::SeqCst), 1);
        // The default is a no-op and an unknown id has nothing to do — both
        // must be safe to call with whatever provider a login pane just ran.
        registry.refresh_availability("fake");
        registry.refresh_availability("no-such-provider");
        assert_eq!(REFRESHED.load(Ordering::SeqCst), 1);
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
    #[test]
    fn agent_role_compatibility_comes_from_descriptor_authority() {
        let mut descriptor = Fake.descriptor();
        descriptor.capabilities = vec!["messages".into()];
        descriptor.sandbox_modes = vec![SandboxMode::WorkspaceWrite, SandboxMode::DangerFullAccess];
        assert!(descriptor_supports_agent_role(
            &descriptor,
            "implementation"
        ));
        assert!(!descriptor_supports_agent_role(&descriptor, "research"));
        assert!(!descriptor_supports_agent_role(&descriptor, "orchestrator"));
        descriptor.capabilities.push("briefings".into());
        assert!(descriptor_supports_agent_role(&descriptor, "orchestrator"));
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
        let cursor = registry
            .descriptors()
            .into_iter()
            .find(|descriptor| descriptor.id == "cursor")
            .expect("cursor adapter is registered");
        assert!(!cursor.supports_sandbox(SandboxMode::ReadOnly));
        assert!(cursor.supports_sandbox(SandboxMode::WorkspaceWrite));
        let grok = registry
            .descriptors()
            .into_iter()
            .find(|descriptor| descriptor.id == "grok")
            .expect("grok adapter is registered");
        assert!(!grok.supports_sandbox(SandboxMode::ReadOnly));
        assert!(grok.supports_sandbox(SandboxMode::WorkspaceWrite));
        for other in registry
            .descriptors()
            .into_iter()
            .filter(|descriptor| !matches!(descriptor.id.as_str(), "opencode" | "cursor" | "grok"))
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
            model_catalog: ModelCatalogDiagnostics::curated(),
        };
        assert!(!unavailable_but_signed_in.available);
        assert_eq!(
            unavailable_but_signed_in.auth_state,
            crate::model::AuthState::SignedIn
        );
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
        assert_eq!(default.actual_model, "opus");
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
            assert_eq!(fallback.actual_model, "opus");
            assert!(fallback
                .warning
                .as_deref()
                .is_some_and(|text| text.contains(hint)));
        }
    }

    #[test]
    fn a_stale_pinned_model_falls_back_to_the_tier_default_with_a_warning() {
        let registry = AdapterRegistry::built_in().unwrap();
        // A model id that has dropped out of the live catalogue must not fail the
        // session start; it resolves to the Standard tier default and warns.
        let stale = registry
            .resolve_pinned_model("claude", "fable-5-1-retired")
            .unwrap();
        assert_eq!(stale.actual_model, "sonnet");
        assert!(stale
            .warning
            .as_deref()
            .is_some_and(|text| text.contains("fable-5-1-retired")));

        // A model still in the catalogue keeps its own tier and never warns.
        let known = registry.resolve_pinned_model("claude", "opus").unwrap();
        assert_eq!(known.actual_model, "opus");
        assert!(known.warning.is_none());
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
        // Waiting for the child does not drain the capture thread. Seeing the
        // first line ("boot") is not evidence that its final error arrived.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while tail
            .snapshot()
            .is_none_or(|text| !text.contains("API error: connection refused"))
            && std::time::Instant::now() < deadline
        {
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
            model_catalog: Arc::new(RwLock::new(model_catalog::resolve(
                "opencode",
                Err("not discovered".into()),
                &[],
                None,
                chrono::Utc::now(),
            ))),
            cache_path: None,
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
        assert!(
            !streams.contains_key("ses_1"),
            "the ended session is dropped"
        );
        assert!(
            streams.contains_key("default"),
            "the shared fallback entry survives per-session teardown"
        );
    }

    #[cfg(unix)]
    #[test]
    fn watchdog_preserves_piped_input_in_posix_shells() {
        use std::io::Write;

        let mut shells = vec![PathBuf::from("/bin/sh")];
        // macOS's /bin/sh is bash; exercise dash there too when available.
        // Linux CI already exercises dash through /bin/sh.
        if let Ok(dash) = which::which("dash") {
            shells.push(dash);
        }
        for shell in shells {
            let mut command = Command::new(&shell);
            command
                .args(["-c", PARENT_WATCHDOG_SCRIPT, "bridge-watchdog", "/bin/cat"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            configure_process_group(&mut command);
            let mut child = command.spawn().unwrap();
            let input = b"first protocol frame\nsecond protocol frame\n";
            // Drop the writer before waiting so the real child observes EOF.
            let write_result = child.stdin.take().unwrap().write_all(input);
            let output = child.wait_with_output().unwrap();
            assert!(write_result.is_ok(), "{} closed its input: {write_result:?}", shell.display());
            assert!(output.status.success(), "{}: {:?}", shell.display(), output);
            assert_eq!(output.stdout, input, "{} discarded the provider's stdin", shell.display());
        }
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
        // PID-1 orphan. `setsid` (Linux, always present via util-linux)
        // gives the watchdog its own process group, as configure_process_group
        // does in production. `set -m` (shell job control) does the same on
        // an interactive shell, but silently no-ops without a controlling
        // TTY — which a CI runner never has and macOS's setsid-less coreutils
        // never provide either — so prefer setsid where it exists and fall
        // back to job control for local development.
        let mut intermediate = Command::new("/bin/sh");
        intermediate
            .env("BRIDGE_WATCHDOG_UNDER_TEST", PARENT_WATCHDOG_SCRIPT)
            .env(
                "BRIDGE_WATCHDOG_INNER",
                format!("/bin/sleep {mate_marker} & exec /bin/sleep {child_marker}"),
            )
            .args([
                "-c",
                "if command -v setsid >/dev/null 2>&1; then pfx=setsid; else set -m; pfx=; fi; $pfx /bin/sh -c 'eval \"$BRIDGE_WATCHDOG_UNDER_TEST\"' bridge-watchdog /bin/sh -c \"$BRIDGE_WATCHDOG_INNER\" & sleep 600",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        configure_process_group(&mut intermediate);
        let mut supervisor = intermediate
            .spawn()
            .expect("intermediate supervisor spawns");

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

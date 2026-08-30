//! One live Agent Client Protocol session: launch, handshake, turn, cancel,
//! reload, shutdown.
//!
//! **Bridge speaks ACP through the protocol's own crate, not a hand-rolled
//! JSON-RPC layer.** Writing the framing by hand would mean re-deriving a
//! versioned schema, a request/response router, batch handling, and
//! cancellation semantics that are already published, tested, and versioned
//! upstream. What is *not* upstream is what any of it means to Bridge, and that
//! is all this module is: capability gating, the normalized event stream, the
//! permission round trip, and a child whose whole process group Bridge owns.
//!
//! **No async runtime, because `bridge-core` does not have one.** The crate is
//! `Send` throughout and pulls in `futures` rather than tokio, so one
//! connection future driven with [`futures::executor::block_on`] on a dedicated
//! thread is the entire executor — the same thread-per-adapter shape
//! `codex_adapter` and `opencode_adapter` already use. The thread is created
//! with an explicit stack size: an unoptimized build needs roughly half a
//! mebibyte of stack for the dispatch chain of a single inbound message, which
//! is more than the fixed stack a platform worker thread would give it, so a
//! debug build would crash on the agent's first message.
//!
//! **The child belongs to Bridge even though the crate can supervise one.** The
//! crate's own `AcpAgent` transport terminates its process group when the
//! connection future drops, which is the right default and the wrong fit here:
//! Bridge needs a shutdown it can call, that closes the input stream first,
//! that allows a grace period, that is idempotent, and whose process id a
//! caller can see. So the spawn comes from the crate — it already makes the
//! child a group leader so a wrapper launcher cannot orphan the real agent —
//! and the teardown comes from [`crate::adapters::terminate_process_group`],
//! the same primitive every other adapter here is reaped by.
//!
//! **Bridge reads the agent's stdout, the crate parses it.** Two lines of
//! defence the crate cannot provide from inside: incoming lines are bounded
//! before they are allocated, and a line that is not shaped like a JSON message
//! is dropped into a bounded noise tail rather than handed to the parser. The
//! second one matters more than it looks — the transport answers a malformed
//! line with a parse error, so an agent whose launcher prints a shell banner
//! would otherwise get one error response per banner line, forever.
//!
//! **An advertised capability is recorded, never assumed.** `session/load` and
//! `session/resume` are gated on what the agent said at initialization, and
//! asking for one it did not advertise fails here rather than on the wire. The
//! session sub-capabilities are presence-signalled empty objects, so support is
//! decided by presence and a capability object that fails to parse degrades to
//! absent instead of failing the handshake — which is the crate's own
//! `DefaultOnError` behaviour, relied on rather than re-implemented.

use crate::{
    acp_events::{
        approval_settled_event, permission_request_event, runtime_failed_event,
        session_update_event, turn_completed_event, unknown_frame_event, AcpReplayLedger,
        AcpTurnOutcome,
    },
    adapters::{terminate_process_group, ShutdownReason},
    agent::NormalizedEvent,
};
use agent_client_protocol::{
    is_incoming_transport_closed,
    schema::{
        v1::{
            CancelNotification, ContentBlock, InitializeRequest, InitializeResponse,
            LoadSessionRequest, NewSessionRequest, PromptRequest, RequestPermissionOutcome,
            RequestPermissionRequest, RequestPermissionResponse, ResumeSessionRequest,
            SelectedPermissionOutcome, SessionConfigOption, SessionConfigOptionValue, SessionId,
            SessionModeState, SessionNotification, SetSessionConfigOptionRequest, TextContent,
        },
        ProtocolVersion,
    },
    AcpAgent, AcpAgentConfig, Agent, Client, ConnectionTo, ErrorCode, Handled, Lines, Responder,
    UntypedMessage,
};
use futures::{
    channel::mpsc as futures_mpsc,
    future::{select, Either},
    io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    StreamExt,
};
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    task::Poll,
    thread,
    time::Duration,
};

/// Zed measured roughly half a mebibyte of stack per inbound message in an
/// unoptimized build, which overflows the fixed stacks the platform gives its
/// own worker threads. Bridge drives the connection on a thread it sizes
/// itself, so the number is stated here rather than inherited.
const CONNECTION_STACK_BYTES: usize = 8 * 1024 * 1024;

/// How long an agent may take to answer `initialize` before Bridge gives up on
/// it. Generous, because a first launch may be paying for a package manager
/// fetching the agent, and bounded, because an executable that accepts the
/// connection and then says nothing must not hang a session start.
pub const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

/// After a failed handshake, wait this long for the child to exit before
/// reporting. A crash on startup reports as an exit status with its stderr
/// rather than as an opaque transport error, which is the difference between a
/// legible failure and a confusing one.
const EXIT_OBSERVATION_GRACE: Duration = Duration::from_millis(250);

/// A single incoming line larger than this is refused rather than buffered. The
/// crate's transport reads a whole line before parsing it, so without a bound
/// here an agent that never emits a newline would grow the desktop process
/// without limit.
const MAX_INCOMING_LINE_BYTES: usize = 8 * 1024 * 1024;

/// Bounded rolling tails. stderr gets the larger budget because it is the only
/// thing a crashed agent leaves behind; the noise tail only has to show enough
/// non-protocol output to recognize what the executable actually was.
const STDERR_TAIL_BYTES: usize = 64 * 1024;
const NOISE_TAIL_BYTES: usize = 4 * 1024;

/// Events buffered for a caller that has not drained yet. Overflow evicts
/// streaming deltas first — their terminal event carries the full content — and
/// counts every eviction, because a silently shortened stream is worse than a
/// reported one.
const EVENT_QUEUE_CAPACITY: usize = 4_096;

/// The event kinds that may be evicted under pressure.
const TRANSIENT_EVENT_KINDS: [&str; 3] = ["message.delta", "reasoning.delta", "tool.progress"];

/// How long a caller waits for the connection thread to finish its own teardown
/// before reaping the process group out from under it. Bounded rather than an
/// open-ended join, so a wedged transport cannot hold a shutdown open.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// The same wait on the failed-handshake path, where there is nothing left to
/// drain and the child is already being killed.
const REAP_TIMEOUT: Duration = Duration::from_secs(5);

/// What a caller asks for when starting an ACP agent. Data only: nothing here
/// is a shell command, and the executable is resolved by the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpLaunch {
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    /// The working directory of the *session*, sent in `session/new`. ACP has
    /// no way to set the child's own working directory, which is why this is
    /// not a spawn parameter.
    pub cwd: PathBuf,
    /// Reported to the agent at initialization so its logs name the client.
    pub client_name: String,
    pub handshake_timeout: Duration,
}

impl AcpLaunch {
    pub fn new(executable: impl Into<PathBuf>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            args: Vec::new(),
            env: BTreeMap::new(),
            cwd: cwd.into(),
            client_name: "bridge".into(),
            handshake_timeout: DEFAULT_HANDSHAKE_TIMEOUT,
        }
    }

    #[must_use]
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    #[must_use]
    pub fn env(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn handshake_timeout(mut self, timeout: Duration) -> Self {
        self.handshake_timeout = timeout;
        self
    }
}

/// One authentication method an agent offered at initialization.
///
/// Recorded so a caller can tell "not signed in" from "cannot sign in here"
/// without a second round trip. Nothing in this slice performs authentication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpAuthMethod {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

/// What the agent said it can do.
///
/// Every field is evidence from the handshake. Bridge never widens it, and the
/// two reload paths are gated on it before a request is built.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AcpCapabilities {
    pub protocol_version: u16,
    /// `session/load`, still carried by the top-level flag rather than the
    /// session sub-capabilities, exactly as the protocol spells it.
    pub load_session: bool,
    /// The presence-signalled session sub-capabilities. An absent object and a
    /// malformed one both mean "not advertised".
    pub resume_session: bool,
    pub list_sessions: bool,
    pub delete_sessions: bool,
    pub close_sessions: bool,
    pub additional_directories: bool,
    pub prompt_images: bool,
    pub prompt_audio: bool,
    pub prompt_embedded_context: bool,
    pub auth_methods: Vec<AcpAuthMethod>,
    pub agent_name: Option<String>,
    pub agent_version: Option<String>,
}

impl AcpCapabilities {
    fn from_initialize(response: &InitializeResponse) -> Self {
        let agent = &response.agent_capabilities;
        let sessions = &agent.session_capabilities;
        Self {
            protocol_version: response.protocol_version.as_u16(),
            load_session: agent.load_session,
            resume_session: sessions.resume.is_some(),
            list_sessions: sessions.list.is_some(),
            delete_sessions: sessions.delete.is_some(),
            close_sessions: sessions.close.is_some(),
            additional_directories: sessions.additional_directories.is_some(),
            prompt_images: agent.prompt_capabilities.image,
            prompt_audio: agent.prompt_capabilities.audio,
            prompt_embedded_context: agent.prompt_capabilities.embedded_context,
            auth_methods: response
                .auth_methods
                .iter()
                .map(|method| AcpAuthMethod {
                    id: method.id().0.to_string(),
                    name: method.name().to_owned(),
                    description: method.description().map(str::to_owned),
                })
                .collect(),
            agent_name: response
                .agent_info
                .as_ref()
                .map(|info| info.name.to_string()),
            agent_version: response
                .agent_info
                .as_ref()
                .map(|info| info.version.to_string()),
        }
    }
}

/// The mode and configuration state a session was opened with.
///
/// Held as the protocol crate's own types rather than a Bridge shape. Models,
/// modes and thought levels are advertised through one generic selector
/// mechanism whose meaning is entirely the agent's: the identifier namespaces
/// differ per agent, several selectors can share a category, and a client that
/// normalizes them into its own vocabulary loses the one thing it must send
/// back unchanged. So this module records what arrived and leaves reading it to
/// the harness that knows the agent.
///
/// Both fields degrade to empty rather than failing the handshake — the crate
/// deserializes them with `DefaultOnError`, so an agent whose selector shape
/// this build cannot parse still opens a session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AcpSessionState {
    pub modes: Option<SessionModeState>,
    pub config_options: Vec<SessionConfigOption>,
}

/// Why an ACP session could not do what was asked. One code per condition, so a
/// caller can act on them without matching on prose — the same shape
/// [`crate::agent_integration::IntegrationError`] already uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcpError {
    /// The executable could not be started at all.
    Launch { reason: String },
    /// The agent accepted the connection and then failed to complete the
    /// handshake in time. Carries whatever non-protocol output it did produce.
    HandshakeTimeout { millis: u64, output: Option<String> },
    /// The handshake reached the agent and came back wrong, or the child died
    /// during it.
    HandshakeFailed {
        reason: String,
        output: Option<String>,
    },
    /// The agent answered with a protocol version Bridge did not ask for.
    ProtocolVersion { requested: u16, offered: u16 },
    /// The agent is installed and talking, and refused to open a session until
    /// someone signs in. Separated from [`Self::HandshakeFailed`] because the
    /// remedy is entirely different and the protocol reserves a code for it:
    /// reported as a crash, a signed-out agent sends a user to look for a bug
    /// that is not there.
    AuthenticationRequired { reason: String },
    /// A method whose capability the agent never advertised. Refused here
    /// rather than sent and rejected: an advertisement is the whole contract,
    /// and speculatively calling past it is how a client learns to guess.
    Unsupported {
        capability: &'static str,
        method: &'static str,
    },
    /// The agent refused to reload a provider session it does not know.
    UnknownProviderSession {
        provider_session_id: String,
        reason: String,
    },
    /// An approval Bridge is not holding a responder for. Answering twice, or
    /// answering one a cancel already settled.
    UnknownApproval { request_id: u64 },
    /// An option id the agent did not offer. Bridge echoes back one of the ids
    /// it was given and never invents one.
    UnknownApprovalOption { request_id: u64, option_id: String },
    /// The connection is gone. Carries the bounded failure context.
    Closed { reason: String },
    /// The agent answered a request with a JSON-RPC error.
    Agent { code: i32, message: String },
    /// The transport failed. Carries a bounded reason, never a transcript.
    Transport { reason: String },
}

impl AcpError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Launch { .. } => "acp_launch_failed",
            Self::HandshakeTimeout { .. } => "acp_handshake_timeout",
            Self::HandshakeFailed { .. } => "acp_handshake_failed",
            Self::ProtocolVersion { .. } => "acp_protocol_version",
            Self::AuthenticationRequired { .. } => "acp_authentication_required",
            Self::Unsupported { .. } => "acp_capability_unsupported",
            Self::UnknownProviderSession { .. } => "acp_unknown_provider_session",
            Self::UnknownApproval { .. } => "acp_unknown_approval",
            Self::UnknownApprovalOption { .. } => "acp_unknown_approval_option",
            Self::Closed { .. } => "acp_connection_closed",
            Self::Agent { .. } => "acp_agent_error",
            Self::Transport { .. } => "acp_transport_failed",
        }
    }
}

impl std::fmt::Display for AcpError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Launch { reason } => write!(formatter, "the agent could not start: {reason}"),
            Self::HandshakeTimeout { millis, output } => {
                write!(formatter, "the agent did not initialize within {millis}ms")?;
                match output {
                    Some(output) => write!(formatter, "; it said: {output}"),
                    None => Ok(()),
                }
            }
            Self::HandshakeFailed { reason, output } => {
                write!(formatter, "the agent failed to initialize: {reason}")?;
                match output {
                    Some(output) => write!(formatter, "; it said: {output}"),
                    None => Ok(()),
                }
            }
            Self::ProtocolVersion {
                requested,
                offered,
            } => write!(
                formatter,
                "Bridge asked for protocol version {requested} and the agent answered with {offered}"
            ),
            Self::AuthenticationRequired { reason } => {
                write!(formatter, "the agent requires authentication: {reason}")
            }
            Self::Unsupported { capability, method } => write!(
                formatter,
                "the agent did not advertise {capability}, so {method} was not called"
            ),
            Self::UnknownProviderSession {
                provider_session_id,
                reason,
            } => write!(
                formatter,
                "the agent does not have the session {provider_session_id}: {reason}"
            ),
            Self::UnknownApproval { request_id } => {
                write!(formatter, "approval {request_id} is no longer outstanding")
            }
            Self::UnknownApprovalOption {
                request_id,
                option_id,
            } => write!(
                formatter,
                "approval {request_id} was not offered the option {option_id}"
            ),
            Self::Closed { reason } => write!(formatter, "the agent is gone: {reason}"),
            Self::Agent { code, message } => {
                write!(formatter, "the agent returned error {code}: {message}")
            }
            Self::Transport { reason } => write!(formatter, "the ACP transport failed: {reason}"),
        }
    }
}

impl std::error::Error for AcpError {}

/// What a reload contributed, after reconciling against the session forest.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AcpReplay {
    /// Events the forest does not already hold, in replay order.
    pub events: Vec<NormalizedEvent>,
    /// Events the forest already had, counted rather than returned.
    pub suppressed: usize,
}

/// Which reload the caller asked for. Two methods, two capabilities, and no
/// substituting one for the other: a reload replays history, a resume does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReloadMode {
    Load,
    Resume,
}

impl ReloadMode {
    const fn capability(self) -> &'static str {
        match self {
            Self::Load => "agentCapabilities.loadSession",
            Self::Resume => "sessionCapabilities.resume",
        }
    }

    const fn method(self) -> &'static str {
        match self {
            Self::Load => "session/load",
            Self::Resume => "session/resume",
        }
    }
}

/// Work the connection thread must do, because it needs the connection's own
/// task to await a response on.
///
/// Cancelling and answering a permission are deliberately *not* here: both must
/// land while a prompt is in flight, and a command queue behind a blocked
/// prompt would deliver them after the turn they were meant to interrupt.
enum AcpCommand {
    Prompt {
        text: String,
        reply: mpsc::SyncSender<Result<AcpTurnOutcome, AcpError>>,
    },
    Reload {
        mode: ReloadMode,
        provider_session_id: String,
        reply: mpsc::SyncSender<Result<Vec<NormalizedEvent>, AcpError>>,
    },
    SetConfigOption {
        option_id: String,
        value: SessionConfigOptionValue,
        reply: mpsc::SyncSender<Result<(), AcpError>>,
    },
    Shutdown,
}

/// A permission the agent is blocked on, and the option ids it offered.
struct ParkedApproval {
    responder: Responder<RequestPermissionResponse>,
    options: Vec<String>,
}

#[derive(Default)]
struct EventQueue {
    events: VecDeque<NormalizedEvent>,
    evicted: u64,
}

impl EventQueue {
    fn push(&mut self, event: NormalizedEvent) {
        if self.events.len() >= EVENT_QUEUE_CAPACITY {
            let transient = self
                .events
                .iter()
                .position(|event| TRANSIENT_EVENT_KINDS.contains(&event.kind.as_str()));
            match transient {
                Some(index) => drop(self.events.remove(index)),
                None => drop(self.events.pop_front()),
            }
            self.evicted += 1;
        }
        self.events.push_back(event);
    }

    fn drain(&mut self) -> Vec<NormalizedEvent> {
        self.events.drain(..).collect()
    }
}

/// A rolling byte tail with a truncation marker, for output that is evidence
/// rather than content.
#[derive(Default)]
struct BoundedTail {
    bytes: VecDeque<u8>,
    truncated: bool,
}

impl BoundedTail {
    fn push(&mut self, bytes: &[u8], limit: usize) {
        for &byte in bytes {
            if self.bytes.len() == limit {
                self.bytes.pop_front();
                self.truncated = true;
            }
            self.bytes.push_back(byte);
        }
    }

    fn snapshot(&self) -> Option<String> {
        if self.bytes.is_empty() {
            return None;
        }
        let text = String::from_utf8_lossy(&self.bytes.iter().copied().collect::<Vec<_>>())
            .trim()
            .to_owned();
        if text.is_empty() {
            return None;
        }
        Some(if self.truncated {
            format!("[truncated] {text}")
        } else {
            text
        })
    }
}

/// Everything the connection thread and the caller both touch.
#[derive(Default)]
struct Shared {
    events: Mutex<EventQueue>,
    /// `Some` while a reload is in flight, so replayed updates are collected
    /// for reconciliation instead of being published as live output.
    replay: Mutex<Option<Vec<NormalizedEvent>>>,
    approvals: Mutex<BTreeMap<u64, ParkedApproval>>,
    next_approval_id: AtomicU64,
    stderr: Mutex<BoundedTail>,
    /// Output the agent wrote to stdout that is not shaped like a protocol
    /// message. Kept because it is usually the reason the handshake failed.
    noise: Mutex<BoundedTail>,
    exit: Mutex<Option<String>>,
    /// Set the moment a caller asks for shutdown, so the connection ending is
    /// reported as a stop rather than as the agent falling over.
    stopping: Mutex<Option<ShutdownReason>>,
    closed: AtomicBool,
}

impl Shared {
    /// Route one event: into the replay buffer while a reload is in flight,
    /// into the live queue otherwise. The buffer is armed before the reload
    /// request is published, so no replayed update can miss it.
    fn publish_event(&self, event: NormalizedEvent) {
        let mut replay = self.replay.lock().expect("acp replay buffer poisoned");
        if let Some(buffer) = replay.as_mut() {
            buffer.push(event);
            return;
        }
        drop(replay);
        self.events
            .lock()
            .expect("acp event queue poisoned")
            .push(event);
    }

    fn failure_context(&self) -> Option<String> {
        let exit = self.exit.lock().expect("acp exit slot poisoned").clone();
        let stderr = self
            .stderr
            .lock()
            .expect("acp stderr tail poisoned")
            .snapshot();
        let noise = self
            .noise
            .lock()
            .expect("acp noise tail poisoned")
            .snapshot();
        let parts: Vec<String> = [exit, stderr, noise].into_iter().flatten().collect();
        (!parts.is_empty()).then(|| parts.join("; "))
    }
}

/// A running ACP agent.
///
/// Cheap to hold and safe to share: everything the caller can ask for either
/// reads shared state or posts to the connection thread, and the child is
/// reaped exactly once however the handle goes away.
pub struct AcpSession {
    process_id: Option<u32>,
    provider_session_id: String,
    capabilities: AcpCapabilities,
    session_state: AcpSessionState,
    shared: Arc<Shared>,
    connection: ConnectionTo<Agent>,
    commands: futures_mpsc::UnboundedSender<AcpCommand>,
    finished: Mutex<Option<mpsc::Receiver<()>>>,
    thread: Mutex<Option<thread::JoinHandle<()>>>,
    shutdown: AtomicBool,
}

impl std::fmt::Debug for AcpSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AcpSession")
            .field("process_id", &self.process_id)
            .field("provider_session_id", &self.provider_session_id)
            .field("capabilities", &self.capabilities)
            .field("closed", &self.is_closed())
            .finish()
    }
}

impl AcpSession {
    /// Launch an agent, negotiate protocol version 1, and open a session.
    ///
    /// Blocks until the agent has answered `initialize` and `session/new`, or
    /// until the handshake timeout expires. Either way the child is accounted
    /// for: a failed handshake reaps the process group before returning.
    pub fn connect(launch: AcpLaunch) -> Result<Self, AcpError> {
        let mut config = AcpAgentConfig::new(launch.executable.clone()).args(launch.args.clone());
        for (name, value) in &launch.env {
            config = config.env(name.clone(), value.clone());
        }
        let (stdin, stdout, stderr, child) =
            AcpAgent::new(config)
                .spawn_process()
                .map_err(|error| AcpError::Launch {
                    reason: error.message,
                })?;
        let process_id = child.id();
        let shared = Arc::new(Shared::default());
        let transport = Lines::new(
            outgoing_lines(stdin),
            incoming_lines(stdout, shared.clone()),
        );
        Self::start(
            launch,
            transport,
            shared,
            Some(SupervisedChild {
                process_id,
                child: Box::new(child),
                stderr: Box::pin(stderr),
            }),
        )
    }

    /// The body of [`Self::connect`], over any transport the protocol crate can
    /// connect through.
    ///
    /// Split out because everything above the pipes — the handshake, the
    /// capability capture, the command loop, the teardown order — is the same
    /// whether the agent is a child process or the far end of an in-process
    /// channel, and only one of those can be driven deterministically.
    fn start<T>(
        launch: AcpLaunch,
        transport: T,
        shared: Arc<Shared>,
        supervision: Option<SupervisedChild>,
    ) -> Result<Self, AcpError>
    where
        T: agent_client_protocol::ConnectTo<Client> + Send + 'static,
    {
        let process_id = supervision.as_ref().map(|child| child.process_id);
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<AcpReady, AcpError>>(1);
        let (finished_tx, finished_rx) = mpsc::sync_channel::<()>(1);
        let (commands_tx, commands_rx) = futures_mpsc::unbounded();

        let thread = thread::Builder::new()
            .name("acp-connection".into())
            .stack_size(CONNECTION_STACK_BYTES)
            .spawn({
                let shared = shared.clone();
                move || {
                    let mut supervision = supervision;
                    let stderr = supervision.as_mut().map(SupervisedChild::take_stderr);
                    futures::executor::block_on(drive_connection(
                        launch,
                        transport,
                        shared.clone(),
                        ready_tx,
                        commands_rx,
                        stderr,
                    ));
                    shared.closed.store(true, Ordering::Release);
                    if let Some(child) = supervision.as_mut() {
                        child.reap(&shared);
                    }
                    let _ = finished_tx.send(());
                }
            })
            .map_err(|error| AcpError::Launch {
                reason: error.to_string(),
            })?;

        let ready = match ready_rx.recv() {
            Ok(ready) => ready,
            Err(_) => Err(AcpError::HandshakeFailed {
                reason: "the connection ended before the handshake completed".into(),
                output: shared.failure_context(),
            }),
        };
        match ready {
            Ok(ready) => Ok(Self {
                process_id,
                provider_session_id: ready.provider_session_id,
                capabilities: ready.capabilities,
                session_state: ready.session_state,
                shared,
                connection: ready.connection,
                commands: commands_tx,
                finished: Mutex::new(Some(finished_rx)),
                thread: Mutex::new(Some(thread)),
                shutdown: AtomicBool::new(false),
            }),
            Err(error) => {
                drop(commands_tx);
                if let Some(process_id) = process_id {
                    terminate_process_group(process_id);
                }
                // Join only a thread that said it finished. The wait is
                // bounded precisely because the transport may be wedged, and
                // joining one that never answered would block a failed
                // handshake forever on exactly the condition the timeout
                // exists to survive — the same reason `shutdown` leaves a
                // wedged connection thread detached.
                if finished_rx.recv_timeout(REAP_TIMEOUT).is_ok() {
                    drop(thread.join());
                }
                Err(error)
            }
        }
    }

    /// The child's process id, or `None` for a session driven over an
    /// in-process transport rather than a spawned agent.
    pub const fn process_id(&self) -> Option<u32> {
        self.process_id
    }

    pub fn provider_session_id(&self) -> &str {
        &self.provider_session_id
    }

    pub const fn capabilities(&self) -> &AcpCapabilities {
        &self.capabilities
    }

    /// What the agent said the session itself starts out offering.
    ///
    /// Separate from [`Self::capabilities`] because it comes from a different
    /// answer: capabilities are what the *agent* can do, session state is what
    /// *this session* was opened with. Recorded verbatim rather than
    /// interpreted — an agent's model and mode identifiers are its own
    /// namespace, and a caller that reconstructs one instead of echoing it
    /// back is inventing a value the agent never offered.
    pub const fn session_state(&self) -> &AcpSessionState {
        &self.session_state
    }

    /// Whether the connection has ended. Exposed so a caller can decide to
    /// relaunch; this module has no reconnection policy of its own.
    pub fn is_closed(&self) -> bool {
        self.shared.closed.load(Ordering::Acquire)
    }

    /// Everything the agent has emitted since the last drain.
    pub fn drain(&self) -> Vec<NormalizedEvent> {
        self.shared
            .events
            .lock()
            .expect("acp event queue poisoned")
            .drain()
    }

    /// Events evicted under queue pressure, so a caller that fell behind can
    /// say so rather than showing a stream with holes in it.
    pub fn evicted_events(&self) -> u64 {
        self.shared
            .events
            .lock()
            .expect("acp event queue poisoned")
            .evicted
    }

    /// What the queue holds and what it has dropped, in the shape the adapter
    /// registry's health surface already reads from every other harness.
    ///
    /// The byte fields stay zero because this queue bounds by item count
    /// rather than bytes — reporting a fabricated size would be worse than
    /// reporting none, and the number that matters here is the eviction count.
    pub fn queue_metrics(&self) -> crate::frame_queue::QueueMetricsSnapshot {
        let queue = self.shared.events.lock().expect("acp event queue poisoned");
        crate::frame_queue::QueueMetricsSnapshot {
            depth: queue.events.len(),
            bytes: 0,
            high_water_bytes: 0,
            dropped_transient: queue.evicted,
        }
    }

    /// Why the runtime is in trouble: the exit status when it has one, its
    /// bounded stderr tail, and any non-protocol output it wrote to stdout.
    pub fn failure_context(&self) -> Option<String> {
        self.shared.failure_context()
    }

    /// Send one user turn and wait for the agent to finish it.
    ///
    /// Returns the stop reason. A cancelled turn returns
    /// [`AcpTurnOutcome::Cancelled`] rather than an error: it is the answer the
    /// protocol requires to a cancel Bridge itself sent.
    pub fn prompt(&self, text: &str) -> Result<AcpTurnOutcome, AcpError> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.commands
            .unbounded_send(AcpCommand::Prompt {
                text: text.to_owned(),
                reply: reply_tx,
            })
            .map_err(|_| self.closed_error())?;
        reply_rx.recv().map_err(|_| self.closed_error())?
    }

    /// Ask the agent to abandon the turn in flight.
    ///
    /// Sends the protocol's cancel notification and answers every outstanding
    /// permission with a cancelled outcome — an agent blocked on one would
    /// never see the cancel otherwise. The process is left alone: cancelling a
    /// turn is not ending a session.
    pub fn cancel(&self) -> Result<(), AcpError> {
        let outcome = self
            .connection
            .send_notification(CancelNotification::new(SessionId::from(
                self.provider_session_id.clone(),
            )))
            .map_err(|error| AcpError::Transport {
                reason: error.message,
            });
        self.cancel_outstanding_approvals();
        outcome
    }

    /// Answer a permission with one of the option ids the agent offered.
    ///
    /// The id is echoed back verbatim. An id the agent did not offer is refused
    /// here rather than sent, because a client that can invent an option id can
    /// invent a decision.
    pub fn answer_approval(&self, request_id: u64, option_id: &str) -> Result<(), AcpError> {
        let parked = {
            let mut approvals = self
                .shared
                .approvals
                .lock()
                .expect("acp approvals poisoned");
            let Some(parked) = approvals.get(&request_id) else {
                return Err(AcpError::UnknownApproval { request_id });
            };
            if !parked.options.iter().any(|offered| offered == option_id) {
                return Err(AcpError::UnknownApprovalOption {
                    request_id,
                    option_id: option_id.to_owned(),
                });
            }
            approvals.remove(&request_id).expect("approval was present")
        };
        parked
            .responder
            .respond(RequestPermissionResponse::new(
                RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                    option_id.to_owned(),
                )),
            ))
            .map_err(|error| AcpError::Transport {
                reason: error.message,
            })?;
        self.shared
            .events
            .lock()
            .expect("acp event queue poisoned")
            .push(approval_settled_event(request_id, "selected"));
        Ok(())
    }

    /// Set one of the session configuration options the agent advertised.
    ///
    /// The option id and the value are the agent's own: a selector's values
    /// live in a namespace this crate has no model of, so both are passed
    /// through untouched. A caller that has not read the option off
    /// [`Self::session_state`] has nothing legitimate to send here.
    pub fn set_config_option(
        &self,
        option_id: &str,
        value: SessionConfigOptionValue,
    ) -> Result<(), AcpError> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.commands
            .unbounded_send(AcpCommand::SetConfigOption {
                option_id: option_id.to_owned(),
                value,
                reply: reply_tx,
            })
            .map_err(|_| self.closed_error())?;
        reply_rx.recv().map_err(|_| self.closed_error())?
    }

    /// Reload a prior provider session, replaying its history.
    ///
    /// Only called when the agent advertised `loadSession`. The replay arrives
    /// as notifications *before* this call returns, so the events it produces
    /// are collected by routing that was installed before the request went out.
    /// They are then reconciled against what the session forest already holds:
    /// history has one owner, and a reload is a re-read of it rather than a
    /// second source of truth.
    pub fn load(
        &self,
        provider_session_id: &str,
        ledger: &mut AcpReplayLedger,
    ) -> Result<AcpReplay, AcpError> {
        self.reload(ReloadMode::Load, provider_session_id, ledger)
    }

    /// Reconnect to a prior provider session without replaying its history.
    ///
    /// A separate capability from [`Self::load`] and never a substitute for it.
    pub fn resume(
        &self,
        provider_session_id: &str,
        ledger: &mut AcpReplayLedger,
    ) -> Result<AcpReplay, AcpError> {
        self.reload(ReloadMode::Resume, provider_session_id, ledger)
    }

    /// Close the input stream, allow a grace period, and terminate the whole
    /// process group.
    ///
    /// Idempotent, and the only path that reaps the child — [`Drop`] calls it
    /// too, so a handle that goes out of scope cannot leave an agent running.
    pub fn shutdown(&self, reason: ShutdownReason) {
        if self.shutdown.swap(true, Ordering::AcqRel) {
            return;
        }
        *self
            .shared
            .stopping
            .lock()
            .expect("acp stopping slot poisoned") = Some(reason);
        self.cancel_outstanding_approvals();
        drop(self.commands.unbounded_send(AcpCommand::Shutdown));
        self.commands.close_channel();
        let finished = self
            .finished
            .lock()
            .expect("acp finished slot poisoned")
            .take();
        let torn_down =
            finished.is_some_and(|finished| finished.recv_timeout(SHUTDOWN_TIMEOUT).is_ok());
        let thread = self.thread.lock().expect("acp thread slot poisoned").take();
        if torn_down {
            if let Some(thread) = thread {
                drop(thread.join());
            }
        } else {
            // The connection thread did not finish its own teardown in time.
            // Reap the group directly so no agent survives, and leave the
            // thread detached rather than joining it: a wedged transport must
            // not be able to hold a shutdown open forever.
            if let Some(process_id) = self.process_id {
                terminate_process_group(process_id);
            }
        }
        self.shared.closed.store(true, Ordering::Release);
    }

    fn reload(
        &self,
        mode: ReloadMode,
        provider_session_id: &str,
        ledger: &mut AcpReplayLedger,
    ) -> Result<AcpReplay, AcpError> {
        let advertised = match mode {
            ReloadMode::Load => self.capabilities.load_session,
            ReloadMode::Resume => self.capabilities.resume_session,
        };
        if !advertised {
            return Err(AcpError::Unsupported {
                capability: mode.capability(),
                method: mode.method(),
            });
        }
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.commands
            .unbounded_send(AcpCommand::Reload {
                mode,
                provider_session_id: provider_session_id.to_owned(),
                reply: reply_tx,
            })
            .map_err(|_| self.closed_error())?;
        let replayed = reply_rx.recv().map_err(|_| self.closed_error())??;
        let before = ledger.suppressed();
        ledger.begin_replay();
        let events = replayed
            .into_iter()
            .filter(|event| ledger.admit(event))
            .collect();
        Ok(AcpReplay {
            events,
            suppressed: ledger.suppressed() - before,
        })
    }

    fn cancel_outstanding_approvals(&self) {
        let outstanding = std::mem::take(
            &mut *self
                .shared
                .approvals
                .lock()
                .expect("acp approvals poisoned"),
        );
        for (request_id, parked) in outstanding {
            drop(parked.responder.respond(RequestPermissionResponse::new(
                RequestPermissionOutcome::Cancelled,
            )));
            self.shared
                .events
                .lock()
                .expect("acp event queue poisoned")
                .push(approval_settled_event(request_id, "cancelled"));
        }
    }

    fn closed_error(&self) -> AcpError {
        AcpError::Closed {
            reason: self
                .shared
                .failure_context()
                .unwrap_or_else(|| "the connection ended".into()),
        }
    }
}

impl Drop for AcpSession {
    fn drop(&mut self) {
        self.shutdown(ShutdownReason::AppShutdown);
    }
}

/// The child a launched session owns beyond its protocol connection: a process
/// group that has to be reaped exactly once, and the stderr that explains why
/// it died.
///
/// Absent for a session driven over an in-process transport, where signalling a
/// process id would mean signalling one that is not ours.
struct SupervisedChild {
    process_id: u32,
    child: Box<async_process::Child>,
    stderr: Pin<Box<dyn AsyncRead + Send>>,
}

impl SupervisedChild {
    /// Hand the stderr stream to the connection future, which is the only place
    /// that can drain it concurrently with the protocol.
    fn take_stderr(&mut self) -> Pin<Box<dyn AsyncRead + Send>> {
        std::mem::replace(&mut self.stderr, Box::pin(futures::io::empty()))
    }

    /// Terminate the process group and record the exit status.
    ///
    /// Reached only after the connection future has returned, which is what
    /// drops the outgoing sink and closes the agent's stdin. The signal
    /// sequence that follows is Bridge's usual one: SIGTERM, a grace period,
    /// then SIGKILL to the whole group so a wrapper launcher's real agent goes
    /// with it.
    fn reap(&mut self, shared: &Arc<Shared>) {
        terminate_process_group(self.process_id);
        if let Ok(Some(status)) = self.child.try_status() {
            let mut exit = shared.exit.lock().expect("acp exit slot poisoned");
            if exit.is_none() {
                *exit = Some(format!("the agent exited with {status}"));
            }
        }
    }
}

/// What the connection thread hands back once the agent is usable.
struct AcpReady {
    capabilities: AcpCapabilities,
    session_state: AcpSessionState,
    provider_session_id: String,
    connection: ConnectionTo<Agent>,
}

async fn drive_connection<T>(
    launch: AcpLaunch,
    transport: T,
    shared: Arc<Shared>,
    ready: mpsc::SyncSender<Result<AcpReady, AcpError>>,
    commands: futures_mpsc::UnboundedReceiver<AcpCommand>,
    stderr: Option<Pin<Box<dyn AsyncRead + Send>>>,
) where
    T: agent_client_protocol::ConnectTo<Client> + Send + 'static,
{
    let protocol = Client
        .builder()
        .name(launch.client_name.clone())
        .on_receive_notification(
            {
                let shared = shared.clone();
                async move |notification: UntypedMessage, cx| {
                    Ok(handle_notification(&shared, notification, cx))
                }
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            {
                let shared = shared.clone();
                async move |request: RequestPermissionRequest, responder, _cx| {
                    park_approval(&shared, &request, responder);
                    Ok(())
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(transport, async |cx: ConnectionTo<Agent>| {
            serve(&launch, &shared, &ready, commands, cx).await
        });

    let drain = async {
        if let Some(stderr) = stderr {
            drain_stderr(stderr, shared.clone()).await;
        }
    };
    let outcome = match select(std::pin::pin!(protocol), std::pin::pin!(drain)).await {
        Either::Left((outcome, _)) => outcome,
        Either::Right(((), protocol)) => protocol.await,
    };
    if let Err(error) = outcome {
        let mut exit = shared.exit.lock().expect("acp exit slot poisoned");
        if exit.is_none() && !is_incoming_transport_closed(&error) {
            *exit = Some(error.message);
        }
    }
    let stopping = *shared.stopping.lock().expect("acp stopping slot poisoned");
    let terminal = match stopping {
        Some(reason) => stopped_event(reason),
        None => runtime_failed_event(
            "acp_connection_closed",
            &shared
                .failure_context()
                .unwrap_or_else(|| "the agent connection ended".into()),
        ),
    };
    shared
        .events
        .lock()
        .expect("acp event queue poisoned")
        .push(terminal);
    // The terminal event is queued before `closed` flips: a consumer whose
    // exit condition is "closed and the queue drained empty" must never be
    // able to observe closed-and-empty while the terminal event is still on
    // its way.
    shared.closed.store(true, Ordering::Release);
}

/// The connection ended because a caller asked it to. Distinguished from a
/// runtime failure so a deliberate stop does not read as a crash.
fn stopped_event(reason: ShutdownReason) -> NormalizedEvent {
    let mut event = NormalizedEvent::new("session.status");
    event.status = Some(reason.as_str().into());
    event.data = serde_json::json!({"shutdownReason": reason.as_str()});
    event
}

/// Handshake, open a session, then serve commands until the caller stops.
async fn serve(
    launch: &AcpLaunch,
    shared: &Arc<Shared>,
    ready: &mpsc::SyncSender<Result<AcpReady, AcpError>>,
    mut commands: futures_mpsc::UnboundedReceiver<AcpCommand>,
    cx: ConnectionTo<Agent>,
) -> Result<(), agent_client_protocol::Error> {
    let opened = match open_session(launch, shared, &cx).await {
        Ok(opened) => opened,
        Err(error) => {
            drop(ready.send(Err(error)));
            return Ok(());
        }
    };
    drop(ready.send(Ok(AcpReady {
        capabilities: opened.capabilities,
        session_state: opened.session_state,
        provider_session_id: opened.session_id.0.to_string(),
        connection: cx.clone(),
    })));

    // The loop ends on either of the two things that can end a session: the
    // caller asking, or the agent's incoming stream reaching EOF. Racing the
    // second one is what stops a dead child from leaving this future parked on
    // a command that will never come -- the crate closes the connection's
    // incoming side but deliberately does not cancel this future for us.
    loop {
        let command = {
            let next = commands.next();
            let closed = cx.incoming_closed();
            match select(std::pin::pin!(next), std::pin::pin!(closed)).await {
                Either::Left((Some(command), _)) => command,
                Either::Left((None, _)) | Either::Right(((), _)) => break,
            }
        };
        match command {
            AcpCommand::Prompt { text, reply } => {
                let outcome = run_turn(shared, &cx, &opened.session_id, &text).await;
                drop(reply.send(outcome));
            }
            AcpCommand::Reload {
                mode,
                provider_session_id,
                reply,
            } => {
                let replayed = run_reload(launch, shared, &cx, mode, &provider_session_id).await;
                drop(reply.send(replayed));
            }
            AcpCommand::SetConfigOption {
                option_id,
                value,
                reply,
            } => {
                let outcome =
                    run_set_config_option(&cx, &opened.session_id, &option_id, value).await;
                drop(reply.send(outcome));
            }
            AcpCommand::Shutdown => break,
        }
    }
    Ok(())
}

async fn run_set_config_option(
    cx: &ConnectionTo<Agent>,
    session_id: &SessionId,
    option_id: &str,
    value: SessionConfigOptionValue,
) -> Result<(), AcpError> {
    cx.send_request(SetSessionConfigOptionRequest::new(
        session_id.clone(),
        option_id.to_owned(),
        value,
    ))
    .block_task()
    .await
    .map(drop)
    .map_err(|error| AcpError::Agent {
        code: error.code.into(),
        message: error.message,
    })
}

struct OpenedSession {
    capabilities: AcpCapabilities,
    session_state: AcpSessionState,
    session_id: SessionId,
}

async fn open_session(
    launch: &AcpLaunch,
    shared: &Arc<Shared>,
    cx: &ConnectionTo<Agent>,
) -> Result<OpenedSession, AcpError> {
    let initialize = cx
        .send_request(InitializeRequest::new(ProtocolVersion::V1))
        .block_task();
    let deadline = async_io::Timer::after(launch.handshake_timeout);
    let (response, deadline) = match select(std::pin::pin!(initialize), deadline).await {
        Either::Left((response, deadline)) => (response, deadline),
        Either::Right((_, _)) => {
            observe_exit().await;
            return Err(AcpError::HandshakeTimeout {
                millis: u64::try_from(launch.handshake_timeout.as_millis()).unwrap_or(u64::MAX),
                output: shared.failure_context(),
            });
        }
    };
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            observe_exit().await;
            return Err(AcpError::HandshakeFailed {
                reason: error.message,
                output: shared.failure_context(),
            });
        }
    };
    let capabilities = AcpCapabilities::from_initialize(&response);
    if capabilities.protocol_version != ProtocolVersion::V1.as_u16() {
        return Err(AcpError::ProtocolVersion {
            requested: ProtocolVersion::V1.as_u16(),
            offered: capabilities.protocol_version,
        });
    }
    let open = cx
        .send_request(NewSessionRequest::new(launch.cwd.clone()))
        .block_task();
    let opened = match select(std::pin::pin!(open), deadline).await {
        Either::Left((opened, _)) => opened,
        Either::Right((_, _)) => {
            observe_exit().await;
            return Err(AcpError::HandshakeTimeout {
                millis: u64::try_from(launch.handshake_timeout.as_millis()).unwrap_or(u64::MAX),
                output: shared.failure_context(),
            });
        }
    }
        .map_err(|error| {
            // The protocol reserves one code for "sign in first", and the
            // message beside it is the agent's own prose. Keyed on the code so
            // a caller can act on the condition rather than pattern-match
            // sentences.
            if i32::from(error.code) == i32::from(ErrorCode::AuthRequired) {
                AcpError::AuthenticationRequired {
                    reason: error.message,
                }
            } else {
                AcpError::HandshakeFailed {
                    reason: error.message,
                    output: shared.failure_context(),
                }
            }
        })?;
    Ok(OpenedSession {
        capabilities,
        session_state: AcpSessionState {
            modes: opened.modes,
            config_options: opened.config_options.unwrap_or_default(),
        },
        session_id: opened.session_id,
    })
}

async fn run_turn(
    shared: &Arc<Shared>,
    cx: &ConnectionTo<Agent>,
    session_id: &SessionId,
    text: &str,
) -> Result<AcpTurnOutcome, AcpError> {
    let response = cx
        .send_request(PromptRequest::new(
            session_id.clone(),
            vec![ContentBlock::Text(TextContent::new(text))],
        ))
        .block_task()
        .await;
    match response {
        Ok(response) => {
            let outcome = AcpTurnOutcome::from_stop_reason(response.stop_reason);
            shared
                .events
                .lock()
                .expect("acp event queue poisoned")
                .push(turn_completed_event(outcome));
            Ok(outcome)
        }
        Err(error) if is_incoming_transport_closed(&error) => Err(AcpError::Closed {
            reason: shared
                .failure_context()
                .unwrap_or_else(|| error.message.clone()),
        }),
        Err(error) => Err(AcpError::Agent {
            code: error.code.into(),
            message: error.message,
        }),
    }
}

async fn run_reload(
    launch: &AcpLaunch,
    shared: &Arc<Shared>,
    cx: &ConnectionTo<Agent>,
    mode: ReloadMode,
    provider_session_id: &str,
) -> Result<Vec<NormalizedEvent>, AcpError> {
    // Routing before publication: the replay sink is armed before the request
    // leaves, so history that arrives while the call is still outstanding lands
    // in it rather than in the live stream or in nothing at all.
    *shared.replay.lock().expect("acp replay buffer poisoned") = Some(Vec::new());
    let session_id = SessionId::from(provider_session_id.to_owned());
    let outcome = match mode {
        ReloadMode::Load => cx
            .send_request(LoadSessionRequest::new(session_id, launch.cwd.clone()))
            .block_task()
            .await
            .map(drop),
        ReloadMode::Resume => cx
            .send_request(ResumeSessionRequest::new(session_id, launch.cwd.clone()))
            .block_task()
            .await
            .map(drop),
    };
    let replayed = shared
        .replay
        .lock()
        .expect("acp replay buffer poisoned")
        .take()
        .unwrap_or_default();
    match outcome {
        Ok(()) => Ok(replayed),
        Err(error) if is_incoming_transport_closed(&error) => Err(AcpError::Closed {
            reason: shared
                .failure_context()
                .unwrap_or_else(|| error.message.clone()),
        }),
        Err(error) => Err(AcpError::UnknownProviderSession {
            provider_session_id: provider_session_id.to_owned(),
            reason: error.message,
        }),
    }
}

/// Give a child that failed the handshake a moment to exit on its own.
///
/// The stderr drain runs concurrently with this future, so the pause is what
/// turns a crash on startup into an exit status with its last words attached
/// rather than an opaque transport error.
async fn observe_exit() {
    async_io::Timer::after(EXIT_OBSERVATION_GRACE).await;
}

fn handle_notification(
    shared: &Arc<Shared>,
    notification: UntypedMessage,
    cx: ConnectionTo<Agent>,
) -> Handled<(UntypedMessage, ConnectionTo<Agent>)> {
    if notification.method != "session/update" {
        // JSON-RPC's own reserved namespace is infrastructure, not agent
        // output; everything else is reported so a vendor notification Bridge
        // has no handler for is visible rather than invisible.
        if !notification.method.starts_with("$/") {
            shared.publish_event(unknown_frame_event(serde_json::json!({
                "method": notification.method,
                "params": notification.params,
            })));
        }
        return Handled::No {
            message: (notification, cx),
            retry: false,
        };
    }
    let event = match serde_json::from_value::<SessionNotification>(notification.params.clone()) {
        Ok(parsed) => session_update_event(&parsed.update),
        Err(_) => unknown_frame_event(notification.params.clone()),
    };
    shared.publish_event(event);
    Handled::Yes
}

fn park_approval(
    shared: &Arc<Shared>,
    request: &RequestPermissionRequest,
    responder: Responder<RequestPermissionResponse>,
) {
    let request_id = shared.next_approval_id.fetch_add(1, Ordering::Relaxed);
    let options = request
        .options
        .iter()
        .map(|option| option.option_id.0.to_string())
        .collect();
    shared
        .approvals
        .lock()
        .expect("acp approvals poisoned")
        .insert(request_id, ParkedApproval { responder, options });
    // Straight to the live queue, never the replay buffer: a parked responder
    // is live state the agent is blocked on, and an approval diverted into a
    // reload's replay would be returned as history — or deduplicated away —
    // while the agent waits forever for an answer no surface ever showed.
    shared
        .events
        .lock()
        .expect("acp event queue poisoned")
        .push(permission_request_event(request_id, request));
}

/// Write one JSON-RPC message per line. Newline-delimited JSON, not
/// `Content-Length` framing.
fn outgoing_lines<W>(
    writer: W,
) -> impl futures::Sink<String, Error = std::io::Error> + Send + 'static
where
    W: AsyncWrite + Send + Unpin + 'static,
{
    futures::sink::unfold(writer, async move |mut writer: W, line: String| {
        let mut bytes = line.into_bytes();
        bytes.push(b'\n');
        writer.write_all(&bytes).await?;
        writer.flush().await?;
        Ok::<_, std::io::Error>(writer)
    })
}

/// Read the agent's stdout as bounded lines, dropping anything that is not
/// shaped like a JSON message.
///
/// Two problems the parser cannot solve from inside. A line is refused once it
/// exceeds [`MAX_INCOMING_LINE_BYTES`], so an agent that never writes a newline
/// cannot grow the process without limit. And a line that does not open with
/// `{` or `[` never reaches the parser at all: the transport answers a
/// malformed line with a parse error, so a launcher that prints a shell banner
/// would otherwise draw one error response per banner line. The dropped output
/// is kept in a bounded tail, because it is usually the explanation for
/// whatever went wrong next.
fn incoming_lines<R>(
    reader: R,
    shared: Arc<Shared>,
) -> impl futures::Stream<Item = std::io::Result<String>> + Send + 'static
where
    R: AsyncRead + Send + Unpin + 'static,
{
    let mut reader = Box::pin(BufReader::new(reader));
    let mut pending: Vec<u8> = Vec::new();
    let mut overlong = false;
    futures::stream::poll_fn(move |context| loop {
        let (chunk, consumed, complete) = {
            let buffer = match Pin::new(&mut reader).poll_fill_buf(context) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Ok(buffer)) => buffer,
                Poll::Ready(Err(error)) => return Poll::Ready(Some(Err(error))),
            };
            if buffer.is_empty() {
                let last = std::mem::take(&mut pending);
                return match admit_line(&shared, last, std::mem::take(&mut overlong)) {
                    Some(line) => Poll::Ready(Some(Ok(line))),
                    None => Poll::Ready(None),
                };
            }
            match buffer.iter().position(|byte| *byte == b'\n') {
                Some(index) => (buffer[..index].to_vec(), index + 1, true),
                None => (buffer.to_vec(), buffer.len(), false),
            }
        };
        Pin::new(&mut reader).consume(consumed);
        if pending.len() + chunk.len() > MAX_INCOMING_LINE_BYTES {
            overlong = true;
            pending.clear();
        } else {
            pending.extend_from_slice(&chunk);
        }
        if !complete {
            continue;
        }
        let line = std::mem::take(&mut pending);
        let was_overlong = std::mem::take(&mut overlong);
        if let Some(line) = admit_line(&shared, line, was_overlong) {
            return Poll::Ready(Some(Ok(line)));
        }
    })
}

/// Whether one complete stdout line is a protocol message, recording it in the
/// noise tail when it is not.
fn admit_line(shared: &Arc<Shared>, line: Vec<u8>, overlong: bool) -> Option<String> {
    if overlong {
        shared.noise.lock().expect("acp noise tail poisoned").push(
            b"[a stdout line over the size limit was refused]\n",
            NOISE_TAIL_BYTES,
        );
        return None;
    }
    if line.is_empty() {
        return None;
    }
    let text = String::from_utf8_lossy(&line).into_owned();
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        let mut noise = shared.noise.lock().expect("acp noise tail poisoned");
        noise.push(trimmed.as_bytes(), NOISE_TAIL_BYTES);
        noise.push(b"\n", NOISE_TAIL_BYTES);
        return None;
    }
    Some(text)
}

/// Drain stderr continuously into a bounded rolling tail, so a chatty agent
/// cannot deadlock on a full pipe and a crashed one still leaves evidence.
async fn drain_stderr<R>(reader: R, shared: Arc<Shared>)
where
    R: AsyncRead + Send + Unpin + 'static,
{
    let mut lines = BufReader::new(reader).lines();
    while let Some(line) = lines.next().await {
        // A line that is not valid UTF-8 is an Err item, and the stream stays
        // readable after it. Stopping the drain on one would refill the pipe
        // and block the agent in write(2) — the exact deadlock this loop
        // exists to prevent — so the bad line is recorded as such and the
        // drain keeps going. A real I/O error still ends it: that pipe is
        // gone, and spinning on it would busy-loop the connection thread.
        let line = match line {
            Ok(line) => line,
            Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                "[a stderr line that was not valid UTF-8 was dropped]".to_owned()
            }
            Err(_) => break,
        };
        let mut tail = shared.stderr.lock().expect("acp stderr tail poisoned");
        tail.push(line.as_bytes(), STDERR_TAIL_BYTES);
        tail.push(b"\n", STDERR_TAIL_BYTES);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::{Channel, TransportFrame};
    use serde_json::{json, Value};

    /// The provider session id every scripted agent hands back.
    const SESSION: &str = "provider-session-1";

    /// The one thing these tests wait on.
    ///
    /// Nothing here asserts how long anything took. The connection runs on its
    /// own thread by design, so an assertion made from the test thread has to
    /// wait for delivery; the bound is generous and a miss fails loudly with
    /// what was actually seen rather than passing quietly.
    const OBSERVATION_TIMEOUT: Duration = Duration::from_secs(20);

    fn wait_until(mut predicate: impl FnMut() -> bool) -> bool {
        let deadline = std::time::Instant::now() + OBSERVATION_TIMEOUT;
        loop {
            if predicate() {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    /// The agent end of an in-process channel, spelled as raw JSON-RPC so a
    /// test can assert on the bytes Bridge actually put on the wire.
    #[derive(Clone)]
    struct AgentWire {
        tx: futures_mpsc::UnboundedSender<TransportFrame>,
    }

    impl AgentWire {
        fn send(&self, message: Value) {
            drop(
                self.tx
                    .unbounded_send(TransportFrame::parse_json(&message.to_string())),
            );
        }

        fn result(&self, request: &Value, result: Value) {
            self.send(json!({"jsonrpc": "2.0", "id": request["id"].clone(), "result": result}));
        }

        fn error(&self, request: &Value, code: i32, message: &str) {
            self.send(json!({
                "jsonrpc": "2.0",
                "id": request["id"].clone(),
                "error": {"code": code, "message": message},
            }));
        }

        fn request(&self, id: &str, method: &str, params: Value) {
            self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        }

        fn notify(&self, method: &str, params: Value) {
            self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
        }

        fn update(&self, update: Value) {
            self.notify(
                "session/update",
                json!({"sessionId": SESSION, "update": update}),
            );
        }

        fn hang_up(&self) {
            self.tx.close_channel();
        }
    }

    struct ScriptedAgent {
        seen: Arc<Mutex<Vec<Value>>>,
        wire: AgentWire,
        thread: Option<thread::JoinHandle<()>>,
    }

    impl ScriptedAgent {
        fn messages(&self) -> Vec<Value> {
            self.seen
                .lock()
                .expect("scripted agent log poisoned")
                .clone()
        }

        fn methods(&self) -> Vec<String> {
            self.messages()
                .iter()
                .filter_map(|message| {
                    message
                        .get("method")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .collect()
        }

        fn params_of(&self, method: &str) -> Option<Value> {
            self.messages()
                .into_iter()
                .find(|message| message.get("method").and_then(Value::as_str) == Some(method))
                .and_then(|message| message.get("params").cloned())
        }

        fn response_to(&self, id: &str) -> Option<Value> {
            self.messages().into_iter().find(|message| {
                message.get("id").and_then(Value::as_str) == Some(id)
                    && (message.get("result").is_some() || message.get("error").is_some())
            })
        }
    }

    impl Drop for ScriptedAgent {
        fn drop(&mut self) {
            // Hang up but do not join: the agent loop only ends once the client
            // half of the channel is dropped, which happens when the session
            // that owns it goes away. Joining here would deadlock on exactly
            // the teardown these tests are trying to reach.
            self.wire.hang_up();
            drop(self.thread.take());
        }
    }

    fn scripted_agent<F>(handler: F) -> (Channel, ScriptedAgent)
    where
        F: FnMut(&Value, &AgentWire) + Send + 'static,
    {
        let (client_side, agent_side) = Channel::duplex();
        let Channel { mut rx, tx } = agent_side;
        let wire = AgentWire { tx };
        let seen = Arc::new(Mutex::new(Vec::new()));
        let thread = thread::Builder::new()
            .name("acp-scripted-agent".into())
            .spawn({
                let wire = wire.clone();
                let seen = seen.clone();
                let mut handler = handler;
                move || {
                    futures::executor::block_on(async move {
                        while let Some(frame) = rx.next().await {
                            let Ok(text) = frame.to_json() else { continue };
                            let Ok(message) = serde_json::from_str::<Value>(&text) else {
                                continue;
                            };
                            seen.lock()
                                .expect("scripted agent log poisoned")
                                .push(message.clone());
                            handler(&message, &wire);
                        }
                    });
                }
            })
            .expect("scripted agent thread");
        (
            client_side,
            ScriptedAgent {
                seen,
                wire,
                thread: Some(thread),
            },
        )
    }

    fn test_launch() -> AcpLaunch {
        AcpLaunch::new("/nonexistent/agent", "/tmp").handshake_timeout(Duration::from_secs(20))
    }

    fn try_connect_scripted<F>(
        launch: AcpLaunch,
        handler: F,
    ) -> (Result<AcpSession, AcpError>, ScriptedAgent)
    where
        F: FnMut(&Value, &AgentWire) + Send + 'static,
    {
        let (transport, agent) = scripted_agent(handler);
        let session = AcpSession::start(launch, transport, Arc::new(Shared::default()), None);
        (session, agent)
    }

    fn connect_with_capabilities(capabilities: Value) -> (AcpSession, ScriptedAgent) {
        let (session, agent) = try_connect_scripted(test_launch(), move |message, wire| {
            answer_handshake(message, wire, &capabilities);
        });
        (
            session.expect("the scripted agent completes the handshake"),
            agent,
        )
    }

    fn connect_scripted<F>(handler: F) -> (AcpSession, ScriptedAgent)
    where
        F: FnMut(&Value, &AgentWire) + Send + 'static,
    {
        let (session, agent) = try_connect_scripted(test_launch(), handler);
        (
            session.expect("the scripted agent completes the handshake"),
            agent,
        )
    }

    /// Answer the two handshake methods, and say whether this message was one.
    fn answer_handshake(message: &Value, wire: &AgentWire, capabilities: &Value) -> bool {
        match message.get("method").and_then(Value::as_str) {
            Some("initialize") => {
                wire.result(
                    message,
                    json!({
                        "protocolVersion": 1,
                        "agentCapabilities": capabilities,
                        "authMethods": [],
                    }),
                );
                true
            }
            Some("session/new") => {
                wire.result(message, json!({"sessionId": SESSION}));
                true
            }
            _ => false,
        }
    }

    /// Drain until `count` events of `kind` have arrived, keeping everything
    /// drained along the way so a later assertion can still see it.
    fn drain_until(session: &AcpSession, kind: &str, count: usize) -> Vec<NormalizedEvent> {
        let mut collected: Vec<NormalizedEvent> = Vec::new();
        let found = wait_until(|| {
            collected.extend(session.drain());
            collected.iter().filter(|event| event.kind == kind).count() >= count
        });
        assert!(found, "expected {count} {kind} events, saw {collected:?}");
        collected
    }

    fn approvals(events: &[NormalizedEvent]) -> Vec<&NormalizedEvent> {
        events
            .iter()
            .filter(|event| event.kind == "approval.requested")
            .collect()
    }

    fn approval_id(event: &NormalizedEvent) -> u64 {
        event
            .data
            .pointer("/requestId")
            .and_then(Value::as_u64)
            .expect("an approval carries its Bridge request id")
    }

    fn approval_options(event: &NormalizedEvent) -> Vec<String> {
        event
            .data
            .pointer("/options")
            .and_then(Value::as_array)
            .expect("an approval carries the offered options")
            .iter()
            .filter_map(|option| option.get("id").and_then(Value::as_str))
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn a_session_handle_can_be_shared_across_the_threads_that_use_it() {
        const fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<AcpSession>();
    }

    #[test]
    fn initialization_records_exactly_what_the_agent_advertised() {
        let (session, _agent) = connect_scripted(|message, wire| {
            if message.get("method").and_then(Value::as_str) == Some("initialize") {
                wire.result(
                    message,
                    json!({
                        "protocolVersion": 1,
                        "agentCapabilities": {
                            "loadSession": true,
                            "promptCapabilities": {"image": true, "embeddedContext": true},
                            "sessionCapabilities": {"resume": {}, "list": {}},
                        },
                        "authMethods": [
                            {"id": "oauth", "name": "Sign in", "description": "in a browser"}
                        ],
                        "agentInfo": {"name": "scripted-agent", "version": "9.9.9"},
                    }),
                );
            } else {
                answer_handshake(message, wire, &json!({}));
            }
        });

        let recorded = session.capabilities();
        assert_eq!(recorded.protocol_version, 1);
        assert!(recorded.load_session);
        assert!(recorded.resume_session);
        assert!(recorded.list_sessions);
        assert!(!recorded.delete_sessions);
        assert!(!recorded.close_sessions);
        assert!(!recorded.additional_directories);
        assert!(recorded.prompt_images);
        assert!(!recorded.prompt_audio);
        assert!(recorded.prompt_embedded_context);
        assert_eq!(recorded.agent_name.as_deref(), Some("scripted-agent"));
        assert_eq!(recorded.agent_version.as_deref(), Some("9.9.9"));
        assert_eq!(recorded.auth_methods.len(), 1);
        assert_eq!(recorded.auth_methods[0].id, "oauth");
        assert_eq!(recorded.auth_methods[0].name, "Sign in");
        assert_eq!(
            recorded.auth_methods[0].description.as_deref(),
            Some("in a browser")
        );
        assert_eq!(session.provider_session_id(), SESSION);
        assert_eq!(session.process_id(), None);
    }

    #[test]
    fn only_protocol_version_one_is_ever_requested() {
        let (_session, agent) = connect_with_capabilities(json!({}));
        let params = agent
            .params_of("initialize")
            .expect("initialize reached the agent");
        assert_eq!(
            params.pointer("/protocolVersion").and_then(Value::as_u64),
            Some(1),
            "a draft protocol version must never be requested"
        );
    }

    #[test]
    fn session_sub_capabilities_are_decided_by_presence_not_truthiness() {
        let (session, _agent) = connect_with_capabilities(json!({
            "sessionCapabilities": {"resume": {}, "list": null, "delete": {}},
        }));
        let recorded = session.capabilities();
        assert!(
            recorded.resume_session,
            "an empty object advertises support"
        );
        assert!(!recorded.list_sessions, "null is not an advertisement");
        assert!(recorded.delete_sessions);
        assert!(
            !recorded.close_sessions,
            "an omitted sub-capability is absent"
        );
    }

    #[test]
    fn a_capability_object_that_fails_to_parse_degrades_to_absent() {
        let (session, _agent) = connect_with_capabilities(json!({
            "sessionCapabilities": {"resume": 7, "close": {}},
        }));
        let recorded = session.capabilities();
        assert!(
            !recorded.resume_session,
            "an unparseable capability is absent, not an error"
        );
        assert!(
            recorded.close_sessions,
            "one bad sibling must not take the rest of the handshake down"
        );
    }

    #[test]
    fn a_method_whose_capability_is_absent_never_reaches_the_wire() {
        let (session, agent) = connect_with_capabilities(json!({}));
        let mut ledger = AcpReplayLedger::default();

        assert_eq!(
            session.load("older", &mut ledger),
            Err(AcpError::Unsupported {
                capability: "agentCapabilities.loadSession",
                method: "session/load",
            })
        );
        assert_eq!(
            session.resume("older", &mut ledger),
            Err(AcpError::Unsupported {
                capability: "sessionCapabilities.resume",
                method: "session/resume",
            })
        );
        let methods = agent.methods();
        assert!(
            !methods
                .iter()
                .any(|method| method.starts_with("session/load")
                    || method.starts_with("session/resume")),
            "a refused capability must not produce a wire round trip, saw {methods:?}"
        );
    }

    #[test]
    fn an_agent_that_never_answers_initialize_times_out() {
        let launch = test_launch().handshake_timeout(Duration::from_millis(250));
        let (session, _agent) = try_connect_scripted(launch, |_message, _wire| {});
        match session {
            Err(AcpError::HandshakeTimeout { millis, .. }) => assert_eq!(millis, 250),
            other => panic!("expected a bounded handshake, got {other:?}"),
        }
    }

    #[test]
    fn session_new_shares_the_initialize_deadline() {
        let launch = test_launch().handshake_timeout(Duration::from_millis(250));
        let (session, agent) = try_connect_scripted(launch, |message, wire| {
            if message.get("method").and_then(Value::as_str) == Some("initialize") {
                wire.result(message, json!({"protocolVersion": 1, "agentCapabilities": {}, "authMethods": []}));
            }
        });
        match session {
            Err(AcpError::HandshakeTimeout { millis, .. }) => assert_eq!(millis, 250),
            other => panic!("expected session/new to time out, got {other:?}"),
        }
        assert!(agent.methods().iter().any(|method| method == "session/new"));
    }

    #[test]
    fn a_streamed_turn_becomes_normalized_events_in_order() {
        let (session, _agent) = connect_scripted(|message, wire| {
            if answer_handshake(message, wire, &json!({})) {
                return;
            }
            if message.get("method").and_then(Value::as_str) == Some("session/prompt") {
                wire.update(json!({
                    "sessionUpdate": "agent_thought_chunk",
                    "content": {"type": "text", "text": "thinking"},
                }));
                wire.update(json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "hello"},
                }));
                wire.update(json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "t1",
                    "title": "Read main.rs",
                    "kind": "read",
                    "status": "in_progress",
                }));
                wire.update(json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "t1",
                    "status": "completed",
                }));
                wire.update(json!({
                    "sessionUpdate": "plan",
                    "entries": [{"content": "step", "priority": "medium", "status": "pending"}],
                }));
                wire.update(
                    json!({"sessionUpdate": "current_mode_update", "currentModeId": "architect"}),
                );
                wire.update(json!({
                    "sessionUpdate": "available_commands_update",
                    "availableCommands": [{"name": "plan", "description": "make one"}],
                }));
                wire.update(json!({"sessionUpdate": "usage_update", "used": 12, "size": 200}));
                wire.result(message, json!({"stopReason": "end_turn"}));
            }
        });

        assert_eq!(
            session.prompt("go").expect("the turn completes"),
            AcpTurnOutcome::EndTurn
        );
        let events = session.drain();
        let kinds: Vec<&str> = events.iter().map(|event| event.kind.as_str()).collect();
        assert_eq!(
            kinds,
            [
                "reasoning.delta",
                "message.delta",
                "tool.started",
                "tool.completed",
                "plan.updated",
                "mode.updated",
                "commands.updated",
                "usage.updated",
                "turn.completed",
            ]
        );
        assert_eq!(events[1].text.as_deref(), Some("hello"));
        assert_eq!(events[1].role.as_deref(), Some("assistant"));
        assert_eq!(events[2].item_id.as_deref(), Some("t1"));
        assert_eq!(events[2].status.as_deref(), Some("inProgress"));
        assert_eq!(events[3].status.as_deref(), Some("completed"));
        assert_eq!(
            events[7]
                .data
                .pointer("/usage/used_tokens")
                .and_then(Value::as_u64),
            Some(12)
        );
        let prompt = _agent
            .params_of("session/prompt")
            .expect("the prompt reached the agent");
        assert_eq!(
            prompt.pointer("/prompt/0/text").and_then(Value::as_str),
            Some("go")
        );
        assert_eq!(
            prompt.pointer("/sessionId").and_then(Value::as_str),
            Some(SESSION)
        );
    }

    #[test]
    fn an_update_this_build_cannot_parse_is_reported_rather_than_dropped() {
        let (session, _agent) = connect_scripted(|message, wire| {
            if answer_handshake(message, wire, &json!({})) {
                return;
            }
            if message.get("method").and_then(Value::as_str) == Some("session/prompt") {
                wire.update(json!({"sessionUpdate": "from_the_future", "payload": 1}));
                wire.result(message, json!({"stopReason": "end_turn"}));
            }
        });

        session.prompt("go").expect("the turn completes");
        let events = session.drain();
        let unknown = events
            .iter()
            .find(|event| event.kind == "provider.unknown")
            .expect("an unparseable update is reported");
        assert_eq!(
            unknown
                .data
                .pointer("/frame/update/sessionUpdate")
                .and_then(Value::as_str),
            Some("from_the_future")
        );
    }

    #[test]
    fn a_vendor_notification_is_reported_rather_than_dropped() {
        let (session, _agent) = connect_scripted(|message, wire| {
            if answer_handshake(message, wire, &json!({})) {
                return;
            }
            if message.get("method").and_then(Value::as_str) == Some("session/prompt") {
                wire.notify("vendor/heartbeat", json!({"beat": 3}));
                wire.result(message, json!({"stopReason": "end_turn"}));
            }
        });

        session.prompt("go").expect("the turn completes");
        let events = session.drain();
        let unknown = events
            .iter()
            .find(|event| event.kind == "provider.unknown")
            .expect("an unhandled notification is reported");
        assert_eq!(
            unknown
                .data
                .pointer("/frame/method")
                .and_then(Value::as_str),
            Some("vendor/heartbeat")
        );
    }

    #[test]
    fn an_unknown_blocking_request_is_answered_method_not_found() {
        let (session, agent) = connect_scripted(|message, wire| {
            if answer_handshake(message, wire, &json!({}))
                && message.get("method").and_then(Value::as_str) == Some("session/new")
            {
                wire.request("vendor-1", "vendor/doTheThing", json!({"why": "blocking"}));
            }
        });

        assert!(
            wait_until(|| agent.response_to("vendor-1").is_some()),
            "an unknown incoming request must be answered, never left waiting"
        );
        let response = agent.response_to("vendor-1").expect("an answer arrived");
        assert_eq!(
            response.pointer("/error/code").and_then(Value::as_i64),
            Some(-32601)
        );
        drop(session);
    }

    /// A prompt that raises one permission and finishes once Bridge answers it.
    fn permission_script(options: Value) -> impl FnMut(&Value, &AgentWire) + Send + 'static {
        let mut prompt_request: Option<Value> = None;
        move |message, wire| {
            if answer_handshake(message, wire, &json!({})) {
                return;
            }
            match message.get("method").and_then(Value::as_str) {
                Some("session/prompt") => {
                    prompt_request = Some(message.clone());
                    wire.request(
                        "perm-1",
                        "session/request_permission",
                        json!({
                            "sessionId": SESSION,
                            "toolCall": {"toolCallId": "t9", "title": "rm -rf build"},
                            "options": options,
                        }),
                    );
                }
                _ => {
                    if message.get("id").and_then(Value::as_str) == Some("perm-1") {
                        if let Some(request) = prompt_request.take() {
                            wire.result(&request, json!({"stopReason": "end_turn"}));
                        }
                    }
                }
            }
        }
    }

    fn two_option_permission() -> Value {
        json!([
            {"optionId": "allow-once", "name": "Allow once", "kind": "allow_once"},
            {"optionId": "reject-once", "name": "Reject", "kind": "reject_once"},
        ])
    }

    #[test]
    fn a_permission_is_answered_with_an_option_id_the_agent_offered() {
        let (session, agent) = connect_scripted(permission_script(two_option_permission()));

        thread::scope(|scope| {
            let turn = scope.spawn(|| session.prompt("go"));
            let events = drain_until(&session, "approval.requested", 1);
            let approval = approvals(&events)[0];
            assert_eq!(approval.item_id.as_deref(), Some("t9"));
            assert_eq!(approval.title.as_deref(), Some("rm -rf build"));
            assert_eq!(approval_options(approval), ["allow-once", "reject-once"]);
            session
                .answer_approval(approval_id(approval), "allow-once")
                .expect("an offered option id is accepted");
            assert_eq!(
                turn.join()
                    .expect("turn thread")
                    .expect("the turn finishes"),
                AcpTurnOutcome::EndTurn
            );
        });

        let response = agent
            .response_to("perm-1")
            .expect("the client answered the permission");
        assert_eq!(
            response["result"],
            json!({"outcome": {"outcome": "selected", "optionId": "allow-once"}}),
            "the answer is an outcome object carrying the agent's own option id"
        );
    }

    #[test]
    fn an_option_id_the_agent_never_offered_is_refused_before_it_reaches_the_wire() {
        let (session, agent) = connect_scripted(permission_script(two_option_permission()));

        thread::scope(|scope| {
            let turn = scope.spawn(|| session.prompt("go"));
            let events = drain_until(&session, "approval.requested", 1);
            let request_id = approval_id(approvals(&events)[0]);

            assert_eq!(
                session.answer_approval(request_id, "approved"),
                Err(AcpError::UnknownApprovalOption {
                    request_id,
                    option_id: "approved".into(),
                }),
                "Bridge must not substitute its own allow/deny vocabulary"
            );
            assert!(
                agent.response_to("perm-1").is_none(),
                "a refused option id must not have reached the agent"
            );

            session
                .answer_approval(request_id, "reject-once")
                .expect("an offered option id is still accepted afterwards");
            assert_eq!(
                turn.join()
                    .expect("turn thread")
                    .expect("the turn finishes"),
                AcpTurnOutcome::EndTurn
            );
        });

        assert_eq!(
            agent.response_to("perm-1").expect("an answer arrived")["result"],
            json!({"outcome": {"outcome": "selected", "optionId": "reject-once"}})
        );
    }

    #[test]
    fn an_approval_can_only_be_answered_once() {
        let (session, _agent) = connect_scripted(permission_script(two_option_permission()));

        thread::scope(|scope| {
            let turn = scope.spawn(|| session.prompt("go"));
            let events = drain_until(&session, "approval.requested", 1);
            let request_id = approval_id(approvals(&events)[0]);
            session
                .answer_approval(request_id, "allow-once")
                .expect("the first answer lands");
            assert_eq!(
                session.answer_approval(request_id, "reject-once"),
                Err(AcpError::UnknownApproval { request_id })
            );
            drop(turn.join().expect("turn thread"));
        });
    }

    #[test]
    fn cancelling_answers_every_outstanding_permission_and_ends_the_turn_cancelled() {
        let mut prompt_request: Option<Value> = None;
        let mut answered = 0_usize;
        let (session, agent) = connect_scripted(move |message, wire| {
            if answer_handshake(message, wire, &json!({})) {
                return;
            }
            if message.get("method").and_then(Value::as_str) == Some("session/prompt") {
                prompt_request = Some(message.clone());
                for id in ["perm-1", "perm-2"] {
                    wire.request(
                        id,
                        "session/request_permission",
                        json!({
                            "sessionId": SESSION,
                            "toolCall": {"toolCallId": id, "title": "touch a file"},
                            "options": [
                                {"optionId": "allow-once", "name": "Allow once", "kind": "allow_once"}
                            ],
                        }),
                    );
                }
                return;
            }
            let answered_permission = matches!(
                message.get("id").and_then(Value::as_str),
                Some("perm-1" | "perm-2")
            );
            if answered_permission {
                answered += 1;
                if answered == 2 {
                    wire.update(json!({
                        "sessionUpdate": "agent_message_chunk",
                        "content": {"type": "text", "text": "after the cancel"},
                    }));
                    if let Some(request) = prompt_request.take() {
                        wire.result(&request, json!({"stopReason": "cancelled"}));
                    }
                }
            }
        });

        thread::scope(|scope| {
            let turn = scope.spawn(|| session.prompt("go"));
            drain_until(&session, "approval.requested", 2);
            session.cancel().expect("the cancel is delivered");
            let outcome = turn
                .join()
                .expect("turn thread")
                .expect("a cancelled turn is a completion, not an error");
            assert_eq!(outcome, AcpTurnOutcome::Cancelled);
            assert!(outcome.is_normal_completion());
        });

        assert!(
            agent
                .methods()
                .iter()
                .any(|method| method == "session/cancel"),
            "cancelling sends the protocol notification"
        );
        for id in ["perm-1", "perm-2"] {
            assert_eq!(
                agent.response_to(id).expect("every permission is answered")["result"],
                json!({"outcome": {"outcome": "cancelled"}}),
                "an outstanding permission is settled with a cancelled outcome"
            );
        }
        let after = session.drain();
        assert!(
            after.iter().any(|event| event.kind == "message.delta"
                && event.text.as_deref() == Some("after the cancel")),
            "updates after the cancel are still accepted, saw {after:?}"
        );
        assert!(after
            .iter()
            .any(|event| event.kind == "turn.completed"
                && event.status.as_deref() == Some("cancelled")));
        assert_eq!(
            after
                .iter()
                .filter(|event| event.kind == "approval.settled"
                    && event.status.as_deref() == Some("cancelled"))
                .count(),
            2
        );
        assert!(
            !session.is_closed(),
            "cancelling a turn does not kill the agent"
        );
    }

    #[test]
    fn every_stop_reason_reaches_the_caller_distinctly() {
        let (session, _agent) = connect_scripted(|message, wire| {
            if answer_handshake(message, wire, &json!({})) {
                return;
            }
            if message.get("method").and_then(Value::as_str) == Some("session/prompt") {
                let reason = message
                    .pointer("/params/prompt/0/text")
                    .and_then(Value::as_str)
                    .unwrap_or("end_turn");
                wire.result(message, json!({"stopReason": reason}));
            }
        });

        let cases = [
            ("end_turn", AcpTurnOutcome::EndTurn),
            ("max_tokens", AcpTurnOutcome::MaxTokens),
            ("max_turn_requests", AcpTurnOutcome::MaxTurnRequests),
            ("refusal", AcpTurnOutcome::Refusal),
            ("cancelled", AcpTurnOutcome::Cancelled),
        ];
        for (reason, expected) in cases {
            assert_eq!(
                session.prompt(reason).expect("the turn completes"),
                expected,
                "{reason} must not be collapsed into another ending"
            );
        }
    }

    #[test]
    fn history_replay_arrives_before_the_reload_returns_and_is_reconciled() {
        let (session, agent) = connect_scripted(|message, wire| {
            if answer_handshake(message, wire, &json!({"loadSession": true})) {
                return;
            }
            if message.get("method").and_then(Value::as_str) == Some("session/load") {
                for text in ["do the thing", "working", "done"] {
                    wire.update(json!({
                        "sessionUpdate": "agent_message_chunk",
                        "content": {"type": "text", "text": text},
                    }));
                }
                wire.result(message, json!({}));
            }
        });

        let mut ledger = AcpReplayLedger::default();
        let replay = session
            .load("older-session", &mut ledger)
            .expect("the reload succeeds");
        assert_eq!(
            replay.events.len(),
            3,
            "no replayed update may be lost to a late subscription"
        );
        assert_eq!(replay.suppressed, 0);
        assert_eq!(
            replay.events[2].text.as_deref(),
            Some("done"),
            "replay keeps its order"
        );
        assert!(
            session
                .drain()
                .iter()
                .all(|event| event.kind != "message.delta"),
            "replayed history must not also appear in the live stream"
        );
        assert_eq!(
            agent
                .params_of("session/load")
                .expect("the reload reached the agent")
                .pointer("/sessionId")
                .and_then(Value::as_str),
            Some("older-session")
        );

        let again = session
            .load("older-session", &mut ledger)
            .expect("a second reload succeeds");
        assert!(
            again.events.is_empty(),
            "a reload of history the forest already holds adds nothing"
        );
        assert_eq!(again.suppressed, 3);
    }

    #[test]
    fn a_reload_of_an_unknown_provider_session_names_the_id() {
        let (session, _agent) = connect_scripted(|message, wire| {
            if answer_handshake(message, wire, &json!({"loadSession": true})) {
                return;
            }
            if message.get("method").and_then(Value::as_str) == Some("session/load") {
                wire.error(message, -32602, "no session with that id");
            }
        });

        let mut ledger = AcpReplayLedger::default();
        let error = session
            .load("ghost-session", &mut ledger)
            .expect_err("an unknown session cannot be reloaded");
        match &error {
            AcpError::UnknownProviderSession {
                provider_session_id,
                reason,
            } => {
                assert_eq!(provider_session_id, "ghost-session");
                assert!(reason.contains("no session with that id"), "{reason}");
            }
            other => panic!("expected an unknown-session error, got {other:?}"),
        }
        assert!(error.to_string().contains("ghost-session"));
    }

    #[test]
    fn resume_reconnects_without_replay_and_is_gated_separately_from_load() {
        let (session, agent) = connect_scripted(|message, wire| {
            if answer_handshake(
                message,
                wire,
                &json!({"sessionCapabilities": {"resume": {}}}),
            ) {
                return;
            }
            if message.get("method").and_then(Value::as_str) == Some("session/resume") {
                wire.result(message, json!({}));
            }
        });

        let mut ledger = AcpReplayLedger::default();
        assert_eq!(
            session.load("older", &mut ledger),
            Err(AcpError::Unsupported {
                capability: "agentCapabilities.loadSession",
                method: "session/load",
            }),
            "resume support does not imply load support"
        );
        let resumed = session
            .resume("older", &mut ledger)
            .expect("resume is advertised");
        assert!(
            resumed.events.is_empty(),
            "a resume reconnects without replaying history"
        );
        assert_eq!(resumed.suppressed, 0);
        assert!(agent
            .methods()
            .iter()
            .any(|method| method == "session/resume"));
    }

    #[test]
    fn a_connection_that_ends_mid_turn_resolves_the_turn_instead_of_erroring_forever() {
        let (session, _agent) = connect_scripted(|message, wire| {
            if answer_handshake(message, wire, &json!({})) {
                return;
            }
            if message.get("method").and_then(Value::as_str) == Some("session/prompt") {
                wire.hang_up();
            }
        });

        let error = session
            .prompt("go")
            .expect_err("a turn whose agent vanished cannot finish");
        assert_eq!(error.code(), "acp_connection_closed");
        assert!(wait_until(|| session.is_closed()));
    }

    #[test]
    fn a_deliberate_stop_is_not_reported_as_a_runtime_failure() {
        let (session, _agent) = connect_with_capabilities(json!({}));
        session.shutdown(ShutdownReason::UserStopped);
        let events = session.drain();
        assert!(
            events.iter().any(|event| event.kind == "session.status"
                && event.status.as_deref() == Some("user_stopped")),
            "a stop the caller asked for is a status, saw {events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|event| event.kind == "error" || event.kind == "runtime.failed"),
            "a deliberate stop must not read as a crash"
        );
    }

    #[test]
    fn a_connection_that_dies_undeliberately_ends_with_the_supervisors_error_shape() {
        let (session, agent) = connect_with_capabilities(json!({}));
        agent.wire.hang_up();
        assert!(wait_until(|| session.is_closed()));
        let events = session.drain();
        assert!(
            events.iter().any(|event| event.kind == "error"
                && event.status.as_deref() == Some("failed")),
            "provider death must land on the failure arm every adapter shares, saw {events:?}"
        );
    }

    #[test]
    fn non_protocol_stdout_never_reaches_the_parser_and_is_kept_as_evidence() {
        let shared = Arc::new(Shared::default());
        assert_eq!(
            admit_line(&shared, b"loading /etc/profile".to_vec(), false),
            None,
            "a shell banner is not a protocol message"
        );
        assert_eq!(admit_line(&shared, b"   ".to_vec(), false), None);
        assert_eq!(
            admit_line(&shared, b"{\"jsonrpc\":\"2.0\"}".to_vec(), false),
            Some("{\"jsonrpc\":\"2.0\"}".to_owned())
        );
        assert_eq!(
            admit_line(&shared, b"[{\"jsonrpc\":\"2.0\"}]".to_vec(), false),
            Some("[{\"jsonrpc\":\"2.0\"}]".to_owned()),
            "a batch is still a protocol message"
        );
        let noise = shared
            .noise
            .lock()
            .expect("noise tail")
            .snapshot()
            .expect("the banner is kept");
        assert!(noise.contains("loading /etc/profile"), "{noise}");
    }

    #[test]
    fn an_incoming_line_over_the_limit_is_refused_rather_than_buffered() {
        let shared = Arc::new(Shared::default());
        let mut input = Vec::new();
        input.extend_from_slice(b"a launcher banner\n");
        input.extend_from_slice(&vec![b'x'; MAX_INCOMING_LINE_BYTES + 1]);
        input.push(b'\n');
        input.extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"method\":\"ping\"}\n");

        let lines: Vec<String> = futures::executor::block_on(
            incoming_lines(futures::io::Cursor::new(input), shared.clone())
                .map(|line| line.expect("the reader does not fail"))
                .collect(),
        );
        assert_eq!(lines, ["{\"jsonrpc\":\"2.0\",\"method\":\"ping\"}"]);
        let noise = shared
            .noise
            .lock()
            .expect("noise tail")
            .snapshot()
            .expect("both refusals are recorded");
        assert!(noise.contains("a launcher banner"), "{noise}");
        assert!(noise.contains("over the size limit"), "{noise}");
    }

    #[test]
    fn the_event_queue_evicts_streaming_deltas_before_durable_events() {
        let mut queue = EventQueue::default();
        queue.push(NormalizedEvent::new("approval.requested"));
        for _ in 0..EVENT_QUEUE_CAPACITY {
            queue.push(NormalizedEvent::new("message.delta"));
        }
        assert_eq!(queue.evicted, 1);
        let drained = queue.drain();
        assert_eq!(drained.len(), EVENT_QUEUE_CAPACITY);
        assert_eq!(
            drained[0].kind, "approval.requested",
            "a durable event outlives the deltas around it"
        );
    }

    #[test]
    fn a_queue_holding_only_durable_events_still_makes_room_and_counts_the_loss() {
        // The eviction preference is transient-first, not transient-only: a
        // queue with nothing streaming in it has to drop the oldest durable
        // event rather than refuse the newest one, and the count is what turns
        // a shortened stream into a reported one instead of a silent hole.
        let mut queue = EventQueue::default();
        for index in 0..EVENT_QUEUE_CAPACITY {
            let mut event = NormalizedEvent::new("tool.started");
            event.item_id = Some(index.to_string());
            queue.push(event);
        }
        assert_eq!(queue.evicted, 0, "nothing was dropped before the queue filled");

        let mut newest = NormalizedEvent::new("tool.started");
        newest.item_id = Some("newest".into());
        queue.push(newest);
        assert_eq!(queue.evicted, 1);

        let drained = queue.drain();
        assert_eq!(drained.len(), EVENT_QUEUE_CAPACITY);
        assert_eq!(
            drained[0].item_id.as_deref(),
            Some("1"),
            "the oldest durable event is the one that made room"
        );
        assert_eq!(
            drained.last().and_then(|event| event.item_id.as_deref()),
            Some("newest"),
            "the newest event is kept, never refused"
        );
    }

    #[test]
    fn a_bounded_tail_keeps_the_last_bytes_and_says_it_truncated() {
        let mut tail = BoundedTail::default();
        assert_eq!(tail.snapshot(), None);
        tail.push(b"first\n", 12);
        tail.push(b"second\n", 12);
        let snapshot = tail.snapshot().expect("the tail has content");
        assert!(snapshot.starts_with("[truncated]"), "{snapshot}");
        assert!(snapshot.contains("second"), "{snapshot}");
        assert!(!snapshot.contains("first"), "{snapshot}");
    }

    fn fake_agent_launch(mode: &str) -> AcpLaunch {
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../testing/fixtures/acp-fake-agent.sh");
        AcpLaunch::new("/bin/sh", std::env::temp_dir())
            .arg(script.to_string_lossy().into_owned())
            .env("BRIDGE_ACP_FAKE_MODE", mode)
    }

    /// A number unique to this test binary and call, used as a sleep duration
    /// so the group mate it spawns can be found by its command line alone.
    fn unique_marker() -> u64 {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        600_000 + u64::from(std::process::id() % 997) * 1_000 + NEXT.fetch_add(1, Ordering::Relaxed)
    }

    fn sleeper_is_running(marker: u64) -> bool {
        std::process::Command::new("pgrep")
            .args(["-f", &format!("^/bin/sleep {marker}$")])
            .output()
            .is_ok_and(|output| output.status.success() && !output.stdout.is_empty())
    }

    /// Whether a pid names a live process. A zombie is not alive: it has been
    /// killed and is only waiting to be collected.
    fn process_is_alive(process_id: u32) -> bool {
        std::process::Command::new("ps")
            .args(["-p", &process_id.to_string(), "-o", "stat="])
            .output()
            .is_ok_and(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .split_whitespace()
                    .next()
                    .is_some_and(|state| !state.starts_with('Z'))
            })
    }

    fn recorded_pid(path: &std::path::Path) -> u32 {
        assert!(
            wait_until(|| path.exists()),
            "the fake agent should record its pid"
        );
        std::fs::read_to_string(path)
            .expect("the pid file is readable")
            .trim()
            .parse()
            .expect("the pid file holds a pid")
    }

    #[test]
    fn shutdown_closes_the_input_stream_and_reaps_the_whole_process_group() {
        let marker = unique_marker();
        let directory = tempfile::tempdir().expect("a temp directory");
        let pid_file = directory.path().join("agent.pid");
        let session = AcpSession::connect(
            fake_agent_launch("serve")
                .env(
                    "BRIDGE_ACP_FAKE_PID_FILE",
                    pid_file.to_string_lossy().into_owned(),
                )
                .env("BRIDGE_ACP_FAKE_GRANDCHILD_SECONDS", marker.to_string()),
        )
        .expect("the fake agent completes the handshake");

        let process_id = session
            .process_id()
            .expect("a spawned agent has a process id");
        assert_eq!(recorded_pid(&pid_file), process_id);
        assert!(
            wait_until(|| sleeper_is_running(marker)),
            "the group mate should be running before shutdown"
        );
        assert_eq!(
            session.prompt("hello").expect("the turn completes"),
            AcpTurnOutcome::EndTurn
        );

        session.shutdown(ShutdownReason::UserStopped);

        assert!(
            !process_is_alive(process_id),
            "the agent must not survive shutdown"
        );
        assert!(
            !sleeper_is_running(marker),
            "no grandchild may survive shutdown"
        );
    }

    #[test]
    fn shutdown_is_idempotent() {
        let session =
            AcpSession::connect(fake_agent_launch("serve")).expect("the handshake completes");
        let process_id = session.process_id().expect("a process id");
        session.shutdown(ShutdownReason::UserStopped);
        session.shutdown(ShutdownReason::Replaced);
        session.shutdown(ShutdownReason::AppShutdown);
        assert!(!process_is_alive(process_id));
        assert!(session.is_closed());
    }

    #[test]
    fn a_failed_initialization_reaps_the_child_and_its_group() {
        let marker = unique_marker();
        let directory = tempfile::tempdir().expect("a temp directory");
        let pid_file = directory.path().join("agent.pid");
        let error = AcpSession::connect(
            fake_agent_launch("silent")
                .env(
                    "BRIDGE_ACP_FAKE_PID_FILE",
                    pid_file.to_string_lossy().into_owned(),
                )
                .env("BRIDGE_ACP_FAKE_GRANDCHILD_SECONDS", marker.to_string())
                .handshake_timeout(Duration::from_millis(500)),
        )
        .expect_err("an agent that says nothing cannot initialize");

        assert_eq!(error.code(), "acp_handshake_timeout");
        let process_id = recorded_pid(&pid_file);
        assert!(
            !process_is_alive(process_id),
            "a failed handshake must not leave the child running"
        );
        assert!(
            !sleeper_is_running(marker),
            "a failed handshake must not leave a grandchild running"
        );
    }

    #[test]
    fn non_protocol_stdout_is_captured_in_the_handshake_failure() {
        let banner = "warning: sourcing /etc/profile";
        let error = AcpSession::connect(
            fake_agent_launch("banner")
                .env("BRIDGE_ACP_FAKE_BANNER", banner)
                .handshake_timeout(Duration::from_millis(500)),
        )
        .expect_err("a banner is not a protocol message");

        match &error {
            AcpError::HandshakeTimeout {
                output: Some(output),
                ..
            } => assert!(output.contains(banner), "{output}"),
            other => panic!("expected the banner in the failure, got {other:?}"),
        }
        assert!(error.to_string().contains(banner));
    }

    #[test]
    fn a_chatty_agent_leaves_a_bounded_stderr_tail_rather_than_a_transcript() {
        let marker = format!("stderr-marker-{}", unique_marker());
        let error = AcpSession::connect(
            fake_agent_launch("noisy_exit")
                .env("BRIDGE_ACP_FAKE_MARKER", marker.clone())
                .env("BRIDGE_ACP_FAKE_STDERR_LINES", "4000")
                .env("BRIDGE_ACP_FAKE_EXIT_CODE", "3")
                .handshake_timeout(Duration::from_secs(20)),
        )
        .expect_err("an agent that exits cannot initialize");

        let output = match &error {
            AcpError::HandshakeFailed {
                output: Some(output),
                ..
            }
            | AcpError::HandshakeTimeout {
                output: Some(output),
                ..
            } => output.clone(),
            other => panic!("expected captured output, got {other:?}"),
        };
        assert!(output.contains(&marker), "the tail keeps the agent's words");
        assert!(
            output.starts_with("[truncated]"),
            "a long stderr is reported as a tail, not a transcript"
        );
        assert!(
            output.len() < STDERR_TAIL_BYTES + 4_096,
            "the failure context stayed bounded at {} bytes",
            output.len()
        );
    }

    #[test]
    fn a_child_that_dies_mid_turn_ends_the_turn_with_its_exit_status_attached() {
        let marker = format!("died-marker-{}", unique_marker());
        let session = AcpSession::connect(
            fake_agent_launch("serve")
                .env("BRIDGE_ACP_FAKE_DIE_ON_PROMPT", "1")
                .env("BRIDGE_ACP_FAKE_MARKER", marker.clone())
                .env("BRIDGE_ACP_FAKE_EXIT_CODE", "9"),
        )
        .expect("the handshake completes before the agent dies");

        let error = session
            .prompt("go")
            .expect_err("a turn cannot finish once the agent is gone");
        assert_eq!(error.code(), "acp_connection_closed");
        assert!(wait_until(|| session.is_closed()));
        assert!(
            wait_until(|| session
                .failure_context()
                .is_some_and(|context| context.contains("exited") && context.contains(&marker))),
            "the failure context should carry the exit status and the stderr tail, got {:?}",
            session.failure_context()
        );
    }
}

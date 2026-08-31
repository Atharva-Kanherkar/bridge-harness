//! The Grok Build harness: finding the vendor's CLI, confirming it speaks
//! the Agent Client Protocol, and running a session through the shared ACP client.
//!
//! **Grok Build runs directly over ACP stdio without a Node sidecar.**
//! Unlike Claude (which requires the Node-based Claude Agent SDK sidecar),
//! Grok Build exposes a native ACP server via `grok agent --no-leader stdio`.
//! Everything after the handshake — turns, streaming, permissions, cancellation,
//! and replay — is [`crate::acp_session`] and [`crate::acp_events`] unchanged.
//!
//! **Strict 1:1 Subprocess Supervision (`--no-leader`).**
//! Bridge supervises the child process lifecycle, signal propagation, and worktree
//! containment. The `--no-leader` argument ensures Grok does not attach to an
//! unmanaged shared background daemon, avoiding worktree lock contention and
//! orphaned processes.
//!
//! **Vendor Authentication Boundary.**
//! Bridge never reads, stores, copies, or persists Grok credentials or cookies.
//! Where a key is configured (`XAI_API_KEY` or `GROK_API_KEY`), it is passed
//! strictly via the child process environment and is redacted from failure context,
//! logs, and events.

use crate::{
    acp_session::{AcpCapabilities, AcpError, AcpLaunch, AcpSession, AcpSessionState},
    adapters::{
        AdapterRuntime, ResumeRequest, ShutdownReason, StartRequest, StartedAdapter, StartupPhase,
    },
    agent::NormalizedEvent,
    binary,
    context_inventory::{
        AdapterContextInventory, ContextInventoryScope, ContextLifecyclePhase, ContextSegmentClass,
        ContextSegmentObservation,
    },
    delegation::WriteMode,
    model::{AdapterDescriptor, AuthState, CapabilityTier, ModelOption},
    BridgeError,
};
use agent_client_protocol::schema::v1::{
    SessionConfigKind, SessionConfigOption, SessionConfigOptionCategory, SessionConfigSelect,
    SessionConfigSelectOptions,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::{BufRead, Read},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex, RwLock,
    },
    thread,
    time::Duration,
};

pub const HARNESS_ID: &str = "grok";
pub const HARNESS_LABEL: &str = "Grok Build";

/// The vendor's executable name.
const PUBLISHED_EXECUTABLE: &str = "grok";

/// The protocol launch arguments.
const ACP_SUBCOMMANDS: [&str; 3] = ["agent", "--no-leader", "stdio"];

/// Environment variables for xAI / Grok API keys.
const KEY_VARIABLES: [&str; 2] = ["XAI_API_KEY", "GROK_API_KEY"];

/// The vendor's sign-in command.
const SIGN_IN_COMMAND: &str = "grok login";

/// How long the probe waits for an ACP handshake.
const PROBE_TIMEOUT: Duration = Duration::from_secs(12);

/// Vendor identification markers.
const VENDOR_MARKERS: [&str; 2] = ["grok", "xai"];

/// How often the event pump moves the session's queue onto the reader.
const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(15);

const CAPABILITIES: &[&str] = crate::builtin_compatibility::CURSOR_CAPABILITIES;

const RESUME_UNAVAILABLE: &str =
    "Grok advertises history replay and not resumption, so Bridge restores its sessions from \
     a checkpoint rather than reconnecting";

/// Where the vendor's CLI was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrokExecutable {
    pub path: PathBuf,
    pub version: String,
}

/// Why the harness is unavailable, stated with actionable remedies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrokUnavailable {
    NotInstalled,
    UnreadableVersion {
        path: PathBuf,
    },
    NotProtocol {
        version: String,
        output: Option<String>,
    },
    Unidentified {
        version: String,
        reported: Option<String>,
    },
    NeedsSignIn {
        version: String,
    },
    ProbeFailed {
        version: String,
        reason: String,
    },
}

impl GrokUnavailable {
    pub fn reason(&self) -> String {
        match self {
            Self::NotInstalled => {
                format!("Grok is not installed; install the {PUBLISHED_EXECUTABLE} CLI or check that it is on PATH")
            }
            Self::UnreadableVersion { path } => {
                format!("Could not read the version of Grok at {}", path.display())
            }
            Self::NotProtocol { version, output } => {
                let detail = output
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| format!(": {s}"))
                    .unwrap_or_default();
                format!(
                    "Grok {version} answered with non-protocol output instead of the agent protocol{detail}"
                )
            }
            Self::Unidentified { version, reported } => match reported {
                Some(name) => format!(
                    "The executable found as {PUBLISHED_EXECUTABLE} ({version}) identified itself as {name:?}, not as Grok"
                ),
                None => format!(
                    "The executable found as {PUBLISHED_EXECUTABLE} ({version}) has not identified itself as Grok"
                ),
            },
            Self::NeedsSignIn { version } => {
                format!("Grok {version} is installed but not signed in; run {SIGN_IN_COMMAND}")
            }
            Self::ProbeFailed { version, reason } => {
                format!("Grok {version} could not start an agent session: {reason}")
            }
        }
    }

    pub const fn auth_state(&self) -> AuthState {
        match self {
            Self::NeedsSignIn { .. } => AuthState::SignedOut,
            _ => AuthState::Unknown,
        }
    }
}

/// What one completed probe established about a Grok build.
#[derive(Debug, Clone, PartialEq)]
pub struct GrokProfile {
    pub executable: PathBuf,
    pub version: String,
    pub agent_name: Option<String>,
    pub load_session: bool,
    pub resume_session: bool,
    pub additional_directories: bool,
    pub prompt_images: bool,
    pub auth_methods: Vec<String>,
    pub modes: Vec<String>,
    pub current_mode: Option<String>,
    pub models: Vec<ModelOption>,
    pub default_model: Option<String>,
}

#[derive(Debug, Clone)]
struct CachedProbe {
    executable: PathBuf,
    version: String,
    outcome: Result<GrokProfile, GrokUnavailable>,
}

impl CachedProbe {
    fn describes(&self, executable: &GrokExecutable) -> bool {
        self.executable == executable.path && self.version == executable.version
    }
}

fn locate_with(
    managed: &dyn Fn() -> Option<PathBuf>,
    resolve: &dyn Fn(&str) -> Option<PathBuf>,
    version_at: &dyn Fn(&Path) -> Option<String>,
) -> Result<GrokExecutable, GrokUnavailable> {
    let path = managed()
        .or_else(|| resolve(PUBLISHED_EXECUTABLE))
        .ok_or(GrokUnavailable::NotInstalled)?;
    let version = version_at(&path)
        .map(|version| version.trim().to_owned())
        .filter(|version| !version.is_empty())
        .ok_or_else(|| GrokUnavailable::UnreadableVersion { path: path.clone() })?;
    Ok(GrokExecutable { path, version })
}

pub fn locate() -> Result<GrokExecutable, GrokUnavailable> {
    locate_with(
        &|| {
            std::env::var_os("BRIDGE_GROK_BIN")
                .map(PathBuf::from)
                .filter(|p| p.is_file() || p.exists())
                .or_else(|| crate::managed_runtime::managed_entrypoint(HARNESS_ID))
        },
        &binary::resolve,
        &binary::version_at,
    )
}

pub fn system_executable() -> Option<PathBuf> {
    std::env::var_os("BRIDGE_GROK_BIN")
        .map(PathBuf::from)
        .filter(|p| p.is_file() || p.exists())
        .or_else(|| binary::resolve(PUBLISHED_EXECUTABLE))
}

pub fn login_executable() -> Result<PathBuf, GrokUnavailable> {
    Ok(locate()?.path)
}

/// Every configured Grok credential, each paired with the variable it came
/// from. All of them are active: the child inherits the parent environment, so
/// a value set under either name reaches the agent and can surface in a
/// diagnostic. Redaction and injection therefore work over the whole set rather
/// than a single first-found key.
fn configured_keys() -> Vec<(&'static str, String)> {
    KEY_VARIABLES
        .iter()
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
                .map(|value| (*name, value))
        })
        .collect()
}

fn launch_for(
    executable: &Path,
    cwd: &Path,
    keys: &[(&'static str, String)],
    timeout: Duration,
) -> AcpLaunch {
    let mut launch = AcpLaunch::new(executable, cwd)
        .arg(ACP_SUBCOMMANDS[0])
        .arg(ACP_SUBCOMMANDS[1])
        .arg(ACP_SUBCOMMANDS[2])
        .handshake_timeout(timeout);
    for (name, value) in keys {
        launch = launch.env(*name, value);
    }
    launch
}

fn redact(text: &str, keys: &[(&'static str, String)]) -> String {
    let mut redacted = text.to_owned();
    for (_, value) in keys {
        if !value.is_empty() {
            redacted = redacted.replace(value.as_str(), "[redacted]");
        }
    }
    redacted
}

fn probe(executable: &GrokExecutable, timeout: Duration) -> Result<GrokProfile, GrokUnavailable> {
    let keys = configured_keys();
    let launch = launch_for(&executable.path, &std::env::temp_dir(), &keys, timeout);
    let session = match AcpSession::connect(launch) {
        Ok(session) => session,
        Err(error) => return Err(classify_probe_failure(&executable.version, &error, &keys)),
    };
    let profile = read_profile(executable, session.capabilities(), session.session_state());
    session.shutdown(ShutdownReason::Completed);
    let profile = profile?;
    if !identifies_as_grok(profile.agent_name.as_deref()) {
        return Err(GrokUnavailable::Unidentified {
            version: executable.version.clone(),
            reported: profile.agent_name,
        });
    }
    Ok(profile)
}

fn identifies_as_grok(agent_name: Option<&str>) -> bool {
    agent_name.is_some_and(|name| {
        let lower = name.to_ascii_lowercase();
        VENDOR_MARKERS.iter().any(|marker| lower.contains(marker))
    })
}

fn classify_probe_failure(
    version: &str,
    error: &AcpError,
    keys: &[(&'static str, String)],
) -> GrokUnavailable {
    match error {
        AcpError::Launch { reason } => GrokUnavailable::ProbeFailed {
            version: version.to_owned(),
            reason: redact(reason, keys),
        },
        AcpError::HandshakeTimeout { output, .. } | AcpError::HandshakeFailed { output, .. } => {
            GrokUnavailable::NotProtocol {
                version: version.to_owned(),
                output: output.as_deref().map(|output| redact(output, keys)),
            }
        }
        AcpError::AuthenticationRequired { .. } => GrokUnavailable::NeedsSignIn {
            version: version.to_owned(),
        },
        other => GrokUnavailable::ProbeFailed {
            version: version.to_owned(),
            reason: redact(&other.to_string(), keys),
        },
    }
}

fn read_profile(
    executable: &GrokExecutable,
    capabilities: &AcpCapabilities,
    state: &AcpSessionState,
) -> Result<GrokProfile, GrokUnavailable> {
    let models = model_options(&state.config_options);
    let default_model = models
        .iter()
        .find(|model| model.default_for_tier && model.tier == CapabilityTier::Standard)
        .or_else(|| models.iter().find(|model| model.default_for_tier))
        .map(|model| model.id.clone());
    Ok(GrokProfile {
        executable: executable.path.clone(),
        version: executable.version.clone(),
        agent_name: capabilities.agent_name.clone(),
        load_session: capabilities.load_session,
        resume_session: capabilities.resume_session,
        additional_directories: capabilities.additional_directories,
        prompt_images: capabilities.prompt_images,
        auth_methods: capabilities
            .auth_methods
            .iter()
            .map(|method| method.id.clone())
            .collect(),
        modes: state
            .modes
            .as_ref()
            .map(|modes| {
                modes
                    .available_modes
                    .iter()
                    .map(|mode| mode.id.0.to_string())
                    .collect()
            })
            .unwrap_or_default(),
        current_mode: state
            .modes
            .as_ref()
            .map(|modes| modes.current_mode_id.0.to_string()),
        models,
        default_model,
    })
}

pub fn advertised_auth_method(profile: &GrokProfile) -> Option<&str> {
    profile.auth_methods.first().map(String::as_str)
}

fn model_options(options: &[SessionConfigOption]) -> Vec<ModelOption> {
    let Some(select) = model_selector(options) else {
        return Vec::new();
    };
    let values = flatten_options(&select.options);
    let current = select.current_value.0.to_string();
    let mut models: Vec<ModelOption> = values
        .into_iter()
        .map(|(id, label)| ModelOption {
            id,
            label,
            tier: CapabilityTier::Standard,
            default_for_tier: false,
        })
        .collect();
    if let Some(index) = models
        .iter()
        .position(|model| model.id == current)
        .or_else(|| (!models.is_empty()).then_some(0))
    {
        models[index].default_for_tier = true;
    }
    models
}

fn model_selector(options: &[SessionConfigOption]) -> Option<&SessionConfigSelect> {
    options.iter().find_map(|option| {
        matches!(option.category, Some(SessionConfigOptionCategory::Model))
            .then(|| match &option.kind {
                SessionConfigKind::Select(select) => Some(select),
                _ => None,
            })
            .flatten()
    })
}

fn flatten_options(options: &SessionConfigSelectOptions) -> Vec<(String, String)> {
    match options {
        SessionConfigSelectOptions::Ungrouped(values) => values
            .iter()
            .map(|value| (value.value.0.to_string(), value.name.clone()))
            .collect(),
        SessionConfigSelectOptions::Grouped(groups) => groups
            .iter()
            .flat_map(|group| {
                group.options.iter().map(move |value| {
                    (
                        value.value.0.to_string(),
                        format!("{} · {}", group.name, value.name),
                    )
                })
            })
            .collect(),
        _ => Vec::new(),
    }
}

pub(crate) fn grok_context_inventory(
    lifecycle_phase: ContextLifecyclePhase,
) -> Result<Vec<AdapterContextInventory>, BridgeError> {
    let observations = || {
        vec![
            ContextSegmentObservation::unavailable(
                ContextSegmentClass::ProviderBaseInstructions,
                match lifecycle_phase {
                    ContextLifecyclePhase::Start => "The agent protocol has no field in which Grok reports the base instructions it opens a session with",
                    ContextLifecyclePhase::Resume => "The agent protocol has no field in which Grok reports the base instructions retained for a reloaded session",
                    ContextLifecyclePhase::PerTurn => "A prompt carries content blocks and nothing about the provider instructions Grok prepends to them",
                },
            ),
            ContextSegmentObservation::unavailable(
                ContextSegmentClass::ToolSchemas,
                "Grok reports tool calls as they happen and never the schemas it presented to the model",
            ),
            ContextSegmentObservation::unavailable(
                ContextSegmentClass::McpDynamicTools,
                "Bridge names the connectors a session may use and Grok does not report which of them reached the model",
            ),
            ContextSegmentObservation::unavailable(
                ContextSegmentClass::SkillsPlugins,
                "Grok advertises slash commands without saying which skills or plugins contribute context behind them",
            ),
            ContextSegmentObservation::unavailable(
                ContextSegmentClass::AgentDefinitions,
                "Grok runs its own sub-agents and the protocol carries no definition of them",
            ),
        ]
    };
    let mut inventories = Vec::new();
    if lifecycle_phase != ContextLifecyclePhase::PerTurn {
        inventories.push(AdapterContextInventory::new(
            HARNESS_ID,
            ContextInventoryScope::Catalog,
            lifecycle_phase,
            observations(),
        )?);
    }
    inventories.push(AdapterContextInventory::new(
        HARNESS_ID,
        ContextInventoryScope::TurnPresented,
        lifecycle_phase,
        observations(),
    )?);
    Ok(inventories)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OfferedOption {
    id: String,
    kind: String,
}

fn option_for_decision<'a>(decision: &str, options: &'a [OfferedOption]) -> Option<&'a str> {
    let by_kind = |wanted: &str| {
        options
            .iter()
            .find(|option| option.kind == wanted)
            .map(|option| option.id.as_str())
    };
    match decision {
        "accept" => by_kind("allow_once"),
        "acceptForSession" => by_kind("allow_always").or_else(|| by_kind("allow_once")),
        "decline" | "cancel" => by_kind("reject_once").or_else(|| by_kind("reject_always")),
        _ => None,
    }
}

pub struct GrokRuntime {
    session: Arc<AcpSession>,
    provider_session_id: String,
    current_turn: Arc<Mutex<Option<String>>>,
    approvals: Arc<Mutex<BTreeMap<u64, Vec<OfferedOption>>>>,
    events: Arc<Mutex<Option<mpsc::Sender<String>>>>,
    pending_instructions: Mutex<Option<String>>,
    pumping: Arc<AtomicBool>,
    context_inventory: Mutex<Vec<AdapterContextInventory>>,
    stopped: bool,
}

impl GrokRuntime {
    fn closed(&self) -> BridgeError {
        BridgeError::Invalid(format!(
            "The Grok session is no longer running{}",
            self.session
                .failure_context()
                .map(|context| format!(": {}", redact(&context, &configured_keys())))
                .unwrap_or_default()
        ))
    }
}

impl AdapterRuntime for GrokRuntime {
    fn process_id(&self) -> u32 {
        self.session.process_id().unwrap_or_default()
    }

    fn provider_session_id(&self) -> &str {
        &self.provider_session_id
    }

    fn current_turn(&self) -> Arc<Mutex<Option<String>>> {
        self.current_turn.clone()
    }

    fn event_queue_metrics(&self) -> Option<crate::frame_queue::QueueMetricsSnapshot> {
        Some(self.session.queue_metrics())
    }

    fn context_inventory(&self) -> Vec<AdapterContextInventory> {
        self.context_inventory.lock().unwrap().clone()
    }

    fn send_turn(&self, text: &str) -> Result<(), BridgeError> {
        if self.session.is_closed() {
            return Err(self.closed());
        }
        let events = self
            .events
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| self.closed())?;
        let turn_id = format!("turn-{}", uuid::Uuid::new_v4());
        let mut started = NormalizedEvent::new("turn.started");
        started.data = json!({"turnId": turn_id});
        events
            .send(encode_event(&started))
            .map_err(|_| self.closed())?;
        let preamble = self.pending_instructions.lock().unwrap().take();
        let text = match preamble {
            Some(instructions) => format!("{instructions}\n\n{text}"),
            None => text.to_owned(),
        };
        let session = self.session.clone();
        thread::Builder::new()
            .name("grok-turn".into())
            .spawn(move || {
                if let Err(error) = session.prompt(&text) {
                    let reason = redact(&error.to_string(), &configured_keys());
                    let event = crate::acp_events::runtime_failed_event(error.code(), &reason);
                    drop(events.send(encode_event(&event)));
                }
            })
            .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        crate::context_inventory::record_runtime_inventory(
            &self.context_inventory,
            grok_context_inventory(ContextLifecyclePhase::PerTurn)?,
        );
        Ok(())
    }

    fn interrupt(&self) -> Result<(), BridgeError> {
        self.session
            .cancel()
            .map_err(|error| BridgeError::Invalid(error.to_string()))
    }

    fn respond(&self, request_id: Value, decision: &str) -> Result<(), BridgeError> {
        self.respond_with_option(request_id, decision, None)
    }

    fn respond_with_option(
        &self,
        request_id: Value,
        decision: &str,
        exact_option_id: Option<&str>,
    ) -> Result<(), BridgeError> {
        let request_id = request_id.as_u64().ok_or_else(|| {
            BridgeError::Invalid(format!("Grok approval id {request_id} is not a number"))
        })?;
        let options = self
            .approvals
            .lock()
            .unwrap()
            .get(&request_id)
            .cloned()
            .ok_or_else(|| {
                BridgeError::Invalid("This Grok approval is no longer outstanding".into())
            })?;
        let option_id = if let Some(exact) = exact_option_id {
            let offered = options.iter().find(|option| option.id == exact).ok_or_else(|| {
                BridgeError::Invalid(format!("Grok did not offer option {exact:?}"))
            })?;
            let compatible = option_for_decision(decision, std::slice::from_ref(offered));
            compatible.ok_or_else(|| {
                BridgeError::Invalid(format!(
                    "Grok option {exact:?} does not represent decision {decision:?}"
                ))
            })?
        } else {
            option_for_decision(decision, &options).ok_or_else(|| {
                BridgeError::Invalid(format!(
                    "Grok did not offer an option for the decision {decision:?}"
                ))
            })?
        };
        self.session
            .answer_approval(request_id, option_id)
            .map_err(|error| BridgeError::Invalid(error.to_string()))
    }

    fn failure_context(&mut self) -> Option<String> {
        self.session
            .failure_context()
            .map(|context| redact(&context, &configured_keys()))
    }

    fn stop(&mut self, reason: ShutdownReason) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        self.session.shutdown(reason);
        self.pumping.store(false, Ordering::Release);
        drop(self.events.lock().unwrap().take());
    }
}

impl Drop for GrokRuntime {
    fn drop(&mut self) {
        self.stop(ShutdownReason::AppShutdown);
    }
}

pub struct GrokEventReader {
    lines: mpsc::Receiver<String>,
    pending: Vec<u8>,
    consumed: usize,
}

impl Read for GrokEventReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let taken = {
            let available = self.fill_buf()?;
            let taken = available.len().min(buffer.len());
            buffer[..taken].copy_from_slice(&available[..taken]);
            taken
        };
        self.consume(taken);
        Ok(taken)
    }
}

impl BufRead for GrokEventReader {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        if self.consumed == self.pending.len() {
            match self.lines.recv() {
                Ok(line) => {
                    self.pending = line.into_bytes();
                    self.pending.push(b'\n');
                    self.consumed = 0;
                }
                Err(_) => {
                    self.pending.clear();
                    self.consumed = 0;
                }
            }
        }
        Ok(&self.pending[self.consumed..])
    }

    fn consume(&mut self, amount: usize) {
        self.consumed = (self.consumed + amount).min(self.pending.len());
    }
}

fn encode_event(event: &NormalizedEvent) -> String {
    json!({
        "kind": event.kind,
        "itemId": event.item_id,
        "role": event.role,
        "status": event.status,
        "title": event.title,
        "text": event.text,
        "data": event.data,
    })
    .to_string()
}

fn decode_event(value: &Value) -> Option<NormalizedEvent> {
    let text_at = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
    Some(NormalizedEvent {
        kind: value.get("kind").and_then(Value::as_str)?.to_owned(),
        item_id: text_at("itemId"),
        role: text_at("role"),
        status: text_at("status"),
        title: text_at("title"),
        text: text_at("text"),
        data: value.get("data").cloned().unwrap_or(Value::Null),
    })
}

fn pump_events(
    session: Arc<AcpSession>,
    events: mpsc::Sender<String>,
    runtime_sender: Arc<Mutex<Option<mpsc::Sender<String>>>>,
    approvals: Arc<Mutex<BTreeMap<u64, Vec<OfferedOption>>>>,
    pumping: Arc<AtomicBool>,
) {
    'pump: loop {
        let drained = session.drain();
        let idle = drained.is_empty();
        for event in &drained {
            record_approval(&approvals, event);
            if events.send(encode_event(event)).is_err() {
                break 'pump;
            }
        }
        if idle {
            if session.is_closed() || !pumping.load(Ordering::Acquire) {
                for event in session.drain() {
                    record_approval(&approvals, &event);
                    if events.send(encode_event(&event)).is_err() {
                        break 'pump;
                    }
                }
                break 'pump;
            }
            thread::sleep(EVENT_POLL_INTERVAL);
        }
    }
    drop(runtime_sender.lock().unwrap().take());
}

fn record_approval(
    approvals: &Arc<Mutex<BTreeMap<u64, Vec<OfferedOption>>>>,
    event: &NormalizedEvent,
) {
    let Some(request_id) = event.data.pointer("/requestId").and_then(Value::as_u64) else {
        return;
    };
    match event.kind.as_str() {
        "permission.requested" => {
            let offered = event
                .data
                .pointer("/options")
                .and_then(Value::as_array)
                .map(|options| {
                    options
                        .iter()
                        .filter_map(|option| {
                            Some(OfferedOption {
                                id: option.get("id").and_then(Value::as_str)?.to_owned(),
                                kind: option
                                    .get("kind")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_owned(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            approvals.lock().unwrap().insert(request_id, offered);
        }
        "approval.settled" => {
            approvals.lock().unwrap().remove(&request_id);
        }
        _ => {}
    }
}

fn launch(
    profile: &GrokProfile,
    cwd: &str,
    model: Option<&str>,
    instructions: Option<&str>,
    on_progress: Option<crate::adapters::StartupProgress<'_>>,
) -> Result<StartedAdapter, BridgeError> {
    let keys = configured_keys();
    if let Some(on_progress) = on_progress {
        on_progress(StartupPhase::Spawning);
    }
    let session = AcpSession::connect(launch_for(
        &profile.executable,
        Path::new(cwd),
        &keys,
        crate::acp_session::DEFAULT_HANDSHAKE_TIMEOUT,
    ))
    .map_err(|error| launch_error(&error, &keys))?;
    if let Some(on_progress) = on_progress {
        on_progress(StartupPhase::Handshake);
    }
    if let Some(model) = model {
        if let Err(error) = apply_model(&session, profile, model) {
            session.shutdown(ShutdownReason::Failed);
            return Err(error);
        }
    }
    let session = Arc::new(session);
    if let Some(on_progress) = on_progress {
        on_progress(StartupPhase::SessionOpen);
    }
    let (sender, receiver) = mpsc::channel();
    let approvals = Arc::new(Mutex::new(BTreeMap::new()));
    let pumping = Arc::new(AtomicBool::new(true));
    let runtime_sender = Arc::new(Mutex::new(Some(sender.clone())));
    thread::Builder::new()
        .name("grok-events".into())
        .spawn({
            let session = session.clone();
            let sender = sender.clone();
            let runtime_sender = runtime_sender.clone();
            let approvals = approvals.clone();
            let pumping = pumping.clone();
            move || pump_events(session, sender, runtime_sender, approvals, pumping)
        })
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let runtime = GrokRuntime {
        provider_session_id: session.provider_session_id().to_owned(),
        session,
        current_turn: Arc::new(Mutex::new(None)),
        approvals,
        events: runtime_sender,
        pending_instructions: Mutex::new(
            instructions
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
        ),
        pumping,
        context_inventory: Mutex::new(grok_context_inventory(ContextLifecyclePhase::Start)?),
        stopped: false,
    };
    Ok(StartedAdapter {
        runtime: Box::new(runtime),
        reader: Box::new(GrokEventReader {
            lines: receiver,
            pending: Vec::new(),
            consumed: 0,
        }),
        startup_messages: Vec::new(),
    })
}

fn apply_model(session: &AcpSession, profile: &GrokProfile, model: &str) -> Result<(), BridgeError> {
    let advertised = profile
        .models
        .iter()
        .find(|option| option.id == model)
        .ok_or_else(|| {
            BridgeError::Invalid(format!(
                "Grok model {model:?} is not one this build of the agent offers"
            ))
        })?;
    let Some(option_id) = model_selector_id(&session.session_state().config_options) else {
        return Err(BridgeError::Invalid(
            "Grok did not advertise a model selector for this session".into(),
        ));
    };
    session
        .set_config_option(
            &option_id,
            agent_client_protocol::schema::v1::SessionConfigOptionValue::value_id(
                advertised.id.clone(),
            ),
        )
        .map_err(|error| BridgeError::Invalid(error.to_string()))
}

fn model_selector_id(options: &[SessionConfigOption]) -> Option<String> {
    options
        .iter()
        .find(|option| {
            matches!(option.category, Some(SessionConfigOptionCategory::Model))
                && matches!(option.kind, SessionConfigKind::Select(_))
        })
        .map(|option| option.id.0.to_string())
}

fn launch_error(error: &AcpError, keys: &[(&'static str, String)]) -> BridgeError {
    match error {
        AcpError::AuthenticationRequired { .. } => {
            BridgeError::Invalid(format!("Grok is not signed in; run {SIGN_IN_COMMAND}"))
        }
        other => BridgeError::Invalid(redact(&other.to_string(), keys)),
    }
}

pub struct GrokAdapter {
    probe: Arc<RwLock<Option<CachedProbe>>>,
    probing: Arc<Mutex<()>>,
    notify: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl GrokAdapter {
    pub fn new(on_discovered: Option<Arc<dyn Fn() + Send + Sync>>) -> Self {
        let adapter = Self {
            probe: Arc::new(RwLock::new(None)),
            probing: Arc::new(Mutex::new(())),
            notify: on_discovered,
        };
        let probe = adapter.probe.clone();
        let probing = adapter.probing.clone();
        let notify = adapter.notify.clone();
        let _ = thread::Builder::new()
            .name("grok-discover".into())
            .spawn(move || {
                drop(store_probe(&probe, &probing));
                if let Some(notify) = notify {
                    notify();
                }
            });
        adapter
    }

    fn profile(&self) -> Result<GrokProfile, GrokUnavailable> {
        // Resolve the binary first, then accept a cached outcome only when it
        // still describes that exact `(path, version)`. Short-circuiting on a
        // cached success before locating would pin `start()` to a stale
        // executable after `BRIDGE_GROK_BIN` changes or the binary is upgraded.
        // Locating is the cheap `--version` read; the expensive ACP handshake
        // is what the `(path, version)` cache spares us here.
        let executable = locate()?;
        self.profile_for(&executable)
    }

    /// Cache decision for an already-located executable: reuse the cached
    /// outcome only when it still describes this exact `(path, version)`,
    /// otherwise re-probe. Split out so cache invalidation is testable without
    /// depending on what `locate()` finds in the ambient environment.
    fn profile_for(&self, executable: &GrokExecutable) -> Result<GrokProfile, GrokUnavailable> {
        if let Some(cached) = self.probe.read().unwrap().as_ref() {
            if cached.describes(executable) {
                return cached.outcome.clone();
            }
        }
        store_probe(&self.probe, &self.probing)
    }

    fn cached(&self) -> Option<Result<GrokProfile, GrokUnavailable>> {
        self.probe
            .read()
            .unwrap()
            .as_ref()
            .map(|cached| cached.outcome.clone())
    }
}

#[cfg(test)]
impl GrokAdapter {
    fn with_probe(outcome: Result<GrokProfile, GrokUnavailable>) -> Self {
        let executable = outcome
            .as_ref()
            .map(|profile| profile.executable.clone())
            .unwrap_or_default();
        let version = outcome
            .as_ref()
            .map(|profile| profile.version.clone())
            .unwrap_or_default();
        Self {
            probe: Arc::new(RwLock::new(Some(CachedProbe {
                executable,
                version,
                outcome,
            }))),
            probing: Arc::new(Mutex::new(())),
            notify: None,
        }
    }
}

fn store_probe(
    cache: &Arc<RwLock<Option<CachedProbe>>>,
    gate: &Arc<Mutex<()>>,
) -> Result<GrokProfile, GrokUnavailable> {
    let _in_flight = gate.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let located = locate();
    if let Ok(executable) = located.as_ref() {
        if let Some(cached) = cache.read().unwrap().as_ref() {
            if cached.describes(executable) {
                return cached.outcome.clone();
            }
        }
    }
    probe_and_record(cache, located)
}

fn refresh_probe(
    cache: &Arc<RwLock<Option<CachedProbe>>>,
    gate: &Arc<Mutex<()>>,
) -> Result<GrokProfile, GrokUnavailable> {
    let _in_flight = gate.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    probe_and_record(cache, locate())
}

fn probe_and_record(
    cache: &Arc<RwLock<Option<CachedProbe>>>,
    located: Result<GrokExecutable, GrokUnavailable>,
) -> Result<GrokProfile, GrokUnavailable> {
    let (executable, version, outcome) = match located {
        Ok(executable) => {
            let outcome = probe(&executable, PROBE_TIMEOUT);
            (executable.path.clone(), executable.version.clone(), outcome)
        }
        Err(reason) => (PathBuf::new(), String::new(), Err(reason)),
    };
    *cache.write().unwrap() = Some(CachedProbe {
        executable,
        version,
        outcome: outcome.clone(),
    });
    outcome
}

impl crate::adapters::HarnessAdapter for GrokAdapter {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn refresh_availability(&self) {
        let probe = self.probe.clone();
        let probing = self.probing.clone();
        let notify = self.notify.clone();
        let _ = thread::Builder::new()
            .name("grok-refresh".into())
            .spawn(move || {
                drop(refresh_probe(&probe, &probing));
                if let Some(notify) = notify {
                    notify();
                }
            });
    }

    fn descriptor(&self) -> AdapterDescriptor {
        let cached = self.cached();
        let profile = cached.as_ref().and_then(|outcome| outcome.as_ref().ok());
        let unavailable = cached.as_ref().and_then(|outcome| outcome.as_ref().err());
        AdapterDescriptor {
            id: HARNESS_ID.into(),
            label: HARNESS_LABEL.into(),
            available: profile.is_some(),
            auth_state: match (profile, unavailable) {
                (Some(_), _) => AuthState::SignedIn,
                (None, Some(reason)) => reason.auth_state(),
                (None, None) => AuthState::Unknown,
            },
            version: profile.map(|profile| profile.version.clone()),
            capabilities: CAPABILITIES.iter().copied().map(str::to_owned).collect(),
            sandbox_modes: crate::builtin_compatibility::CURSOR_SANDBOXES.to_vec(),
            unavailable_reason: profile.is_none().then(|| match unavailable {
                Some(reason) => reason.reason(),
                None if binary::resolve(PUBLISHED_EXECUTABLE).is_none() => {
                    GrokUnavailable::NotInstalled.reason()
                }
                None => "Grok has not finished starting".into(),
            }),
            models: profile
                .map(|profile| profile.models.clone())
                .unwrap_or_default(),
            default_model: profile.and_then(|profile| profile.default_model.clone()),
        }
    }

    fn start(&self, request: StartRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        if request.read_only_sandbox.is_some()
            || matches!(request.write_mode, Some(WriteMode::ReadOnly))
        {
            return Err(BridgeError::Invalid(
                "Grok read-only workers are unsupported because its CLI is spawned by the agent \
                 protocol client rather than inside the offline sandbox; refusing to start without \
                 isolation"
                    .into(),
            ));
        }
        if request.briefing.is_some() {
            return Err(BridgeError::Invalid(
                "Grok cannot run a briefing: it has no certified permission representation, so \
                 an empty tool scope cannot be enforced on it".into(),
            ));
        }
        let profile = self
            .profile()
            .map_err(|reason| BridgeError::Invalid(reason.reason()))?;
        launch(
            &profile,
            request.cwd,
            request.model,
            request.instructions,
            request.on_progress,
        )
    }

    /// Unreachable while [`Self::supports_native_resume`] is false — the
    /// registry refuses the call before it reaches here — and stated rather
    /// than left to a panic so the reason a Grok session returns through a
    /// checkpoint is legible where a reader looks for it.
    fn resume(&self, _request: ResumeRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        Err(BridgeError::Invalid(RESUME_UNAVAILABLE.into()))
    }

    /// Grok is restored from a checkpoint, not reconnected natively, for the
    /// same two independent reasons the Cursor adapter documents.
    ///
    /// The `resume_session` flag recorded on the negotiated profile is the
    /// agent's advertisement, not a green light Bridge can act on: the shared
    /// ACP client scopes every turn to the session its handshake opened, so it
    /// has no reconnect-and-rebind path a `resume()` could drive today, and
    /// `session/load` is history replay rather than resumption. Wiring true
    /// native `session/resume` — the throwaway-`session/new`-then-rebind dance
    /// the shared layer would need — is a follow-up that must be validated
    /// against a live Grok build advertising the capability, not asserted from
    /// an offline handshake. Until then Bridge hands the session over at a
    /// checkpoint, which is what it can actually do, and this returns false so
    /// the registry never offers a resume Bridge cannot perform.
    fn supports_native_resume(&self) -> bool {
        false
    }

    fn normalize(&self, value: &Value) -> Vec<NormalizedEvent> {
        decode_event(value).into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::HarnessAdapter;

    fn fixture_cli() -> PathBuf {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest_dir
            .parent()
            .unwrap()
            .join("testing/fixtures/grok-fake-cli.sh")
    }

    #[allow(dead_code)]
    struct FakeGrokCli {
        mode: &'static str,
        version: &'static str,
        agent_name: &'static str,
    }

    #[allow(dead_code)]
    impl FakeGrokCli {
        const fn speaking_protocol() -> Self {
            Self {
                mode: "protocol",
                version: "1.0.4-e2b819f",
                agent_name: "Grok Build",
            }
        }

        const fn mode(mut self, mode: &'static str) -> Self {
            self.mode = mode;
            self
        }

        const fn version(mut self, version: &'static str) -> Self {
            self.version = version;
            self
        }

        const fn agent_name(mut self, agent_name: &'static str) -> Self {
            self.agent_name = agent_name;
            self
        }

        fn install(&self, directory: &Path, name: &str) -> PathBuf {
            let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../testing/fixtures/grok-fake-cli.sh");
            let path = directory.join(name);
            std::fs::write(
                &path,
                format!(
                    "#!/bin/sh\n\
                     BRIDGE_GROK_FAKE_MODE='{}' \\\n\
                     BRIDGE_GROK_FAKE_VERSION='{}' \\\n\
                     BRIDGE_GROK_FAKE_AGENT_NAME='{}' \\\n\
                     exec /bin/sh '{}' \"$@\"\n",
                    self.mode,
                    self.version,
                    self.agent_name,
                    fixture.display()
                ),
            )
            .expect("the fake CLI is written");
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("the fake CLI is executable");
            path
        }
    }

    #[test]
    fn fake_cli_start_and_turn_stream() {
        let temp_dir = tempfile::tempdir().unwrap();
        let exec_path = FakeGrokCli::speaking_protocol().install(temp_dir.path(), "grok");

        let executable = GrokExecutable {
            path: exec_path.clone(),
            version: "1.0.4-e2b819f".into(),
        };
        let profile = probe(&executable, Duration::from_secs(5)).unwrap();
        let adapter = GrokAdapter::with_probe(Ok(profile));

        // start() re-locates the binary to validate the cache is still current;
        // point discovery at the fake CLI so its `(path, version)` matches the
        // seeded probe and the cached profile is reused rather than re-probed.
        std::env::set_var("BRIDGE_GROK_BIN", &exec_path);

        let worktree = tempfile::tempdir().unwrap();
        let started = adapter.start(StartRequest {
            cwd: worktree.path().to_str().unwrap(),
            model: Some("grok-code"),
            effort: None,
            instructions: Some("You are a helpful assistant"),
            write_mode: None,
            read_only_sandbox: None,
            briefing: None,
            on_progress: None,
        });
        std::env::remove_var("BRIDGE_GROK_BIN");
        let mut started = started.unwrap();

        started.runtime.send_turn("Hello Grok").unwrap();

        let mut lines = Vec::new();
        let mut line = String::new();
        while started.reader.read_line(&mut line).unwrap() > 0 {
            let val: Value = serde_json::from_str(line.trim()).unwrap();
            let event = decode_event(&val).unwrap();
            lines.push(event.kind);
            if lines.contains(&"turn.started".to_string()) && lines.len() >= 2 {
                break;
            }
            line.clear();
        }

        assert!(lines.contains(&"turn.started".to_string()));
        started.runtime.stop(ShutdownReason::Completed);
    }

    #[test]
    fn locate_finds_grok_on_path() {
        let managed = || None;
        let resolve = |name: &str| {
            if name == "grok" {
                Some(PathBuf::from("/usr/local/bin/grok"))
            } else {
                None
            }
        };
        let version_at = |_path: &Path| Some("1.0.4".into());

        let executable = locate_with(&managed, &resolve, &version_at).unwrap();
        assert_eq!(executable.path, PathBuf::from("/usr/local/bin/grok"));
        assert_eq!(executable.version, "1.0.4");
    }

    #[test]
    fn locate_reports_not_installed_when_missing() {
        let managed = || None;
        let resolve = |_name: &str| None;
        let version_at = |_path: &Path| None;

        let err = locate_with(&managed, &resolve, &version_at).unwrap_err();
        assert_eq!(err, GrokUnavailable::NotInstalled);
    }

    #[test]
    fn locate_reports_unreadable_version() {
        let managed = || None;
        let resolve = |name: &str| {
            if name == "grok" {
                Some(PathBuf::from("/bin/grok"))
            } else {
                None
            }
        };
        let version_at = |_path: &Path| None;

        let err = locate_with(&managed, &resolve, &version_at).unwrap_err();
        assert_eq!(
            err,
            GrokUnavailable::UnreadableVersion {
                path: PathBuf::from("/bin/grok")
            }
        );
    }

    #[test]
    fn launch_for_constructs_no_leader_args_and_env() {
        let exec = Path::new("/bin/grok");
        let cwd = Path::new("/tmp/worktree");
        let keys = vec![("XAI_API_KEY", "xai-secret-key-123".to_string())];
        let launch = launch_for(exec, cwd, &keys, Duration::from_secs(5));

        assert_eq!(launch.executable, PathBuf::from("/bin/grok"));
        assert_eq!(launch.cwd, PathBuf::from("/tmp/worktree"));
        assert_eq!(launch.args, vec!["agent", "--no-leader", "stdio"]);
        assert_eq!(
            launch.env.get("XAI_API_KEY"),
            Some(&"xai-secret-key-123".to_string())
        );
    }

    #[test]
    fn launch_for_injects_every_configured_key_under_its_own_name() {
        let exec = Path::new("/bin/grok");
        let cwd = Path::new("/tmp/worktree");
        let keys = vec![
            ("XAI_API_KEY", "xai-value".to_string()),
            ("GROK_API_KEY", "grok-value".to_string()),
        ];
        let launch = launch_for(exec, cwd, &keys, Duration::from_secs(5));

        assert_eq!(launch.env.get("XAI_API_KEY"), Some(&"xai-value".to_string()));
        assert_eq!(launch.env.get("GROK_API_KEY"), Some(&"grok-value".to_string()));
    }

    #[test]
    fn redact_sanitizes_configured_keys() {
        let one = vec![("XAI_API_KEY", "xai-secret-12345".to_string())];
        assert_eq!(
            redact("Error with token xai-secret-12345 in output", &one),
            "Error with token [redacted] in output"
        );
        assert_eq!(redact("Normal error message", &one), "Normal error message");
        assert_eq!(redact("Normal error message", &[]), "Normal error message");
    }

    #[test]
    fn redact_sanitizes_every_active_key_when_both_are_set() {
        // Both variables set to distinct values: a diagnostic can carry either,
        // so redaction must strip both, not just the first-found key.
        let keys = vec![
            ("XAI_API_KEY", "xai-secret-aaa".to_string()),
            ("GROK_API_KEY", "grok-secret-bbb".to_string()),
        ];
        assert_eq!(
            redact("leaked xai-secret-aaa and grok-secret-bbb here", &keys),
            "leaked [redacted] and [redacted] here"
        );
    }

    #[test]
    fn cached_probe_describes_exact_binary() {
        let probe = CachedProbe {
            executable: PathBuf::from("/bin/grok"),
            version: "1.0.0".into(),
            outcome: Err(GrokUnavailable::NotInstalled),
        };

        let match_exec = GrokExecutable {
            path: PathBuf::from("/bin/grok"),
            version: "1.0.0".into(),
        };
        let diff_version = GrokExecutable {
            path: PathBuf::from("/bin/grok"),
            version: "1.0.1".into(),
        };
        let diff_path = GrokExecutable {
            path: PathBuf::from("/usr/bin/grok"),
            version: "1.0.0".into(),
        };

        assert!(probe.describes(&match_exec));
        assert!(!probe.describes(&diff_version));
        assert!(!probe.describes(&diff_path));
    }

    #[test]
    fn profile_reuses_cache_only_for_the_same_binary() {
        let cached_profile = GrokProfile {
            executable: PathBuf::from("/synthetic/grok"),
            version: "1.0.0-cached".into(),
            agent_name: Some("grok".into()),
            load_session: true,
            resume_session: false,
            additional_directories: false,
            prompt_images: false,
            auth_methods: Vec::new(),
            modes: Vec::new(),
            current_mode: None,
            models: Vec::new(),
            default_model: None,
        };
        let adapter = GrokAdapter::with_probe(Ok(cached_profile));

        // Same (path, version): the cached success is reused verbatim.
        let same = GrokExecutable {
            path: PathBuf::from("/synthetic/grok"),
            version: "1.0.0-cached".into(),
        };
        assert_eq!(adapter.profile_for(&same).unwrap().version, "1.0.0-cached");

        // Binary upgraded in place (version changed): the stale cached profile
        // must not be returned. Re-probing the synthetic path cannot reproduce
        // the cached version, so any outcome here is an error or a freshly read
        // profile — never the stale success the old short-circuit returned.
        let upgraded = GrokExecutable {
            path: PathBuf::from("/synthetic/grok"),
            version: "1.0.1-upgraded".into(),
        };
        assert!(
            adapter.profile_for(&upgraded).map(|profile| profile.version)
                != Ok("1.0.0-cached".to_string()),
            "cache invalidation must not return the stale profile"
        );
    }

    #[test]
    fn option_for_decision_maps_standard_choices() {
        let options = vec![
            OfferedOption {
                id: "allow-once-id".into(),
                kind: "allow_once".into(),
            },
            OfferedOption {
                id: "allow-always-id".into(),
                kind: "allow_always".into(),
            },
            OfferedOption {
                id: "reject-once-id".into(),
                kind: "reject_once".into(),
            },
        ];

        assert_eq!(option_for_decision("accept", &options), Some("allow-once-id"));
        assert_eq!(
            option_for_decision("acceptForSession", &options),
            Some("allow-always-id")
        );
        assert_eq!(option_for_decision("decline", &options), Some("reject-once-id"));
        assert_eq!(option_for_decision("cancel", &options), Some("reject-once-id"));
        assert_eq!(option_for_decision("unknown", &options), None);
    }

    #[test]
    fn grok_context_inventory_covers_all_phases() {
        let start_inv = grok_context_inventory(ContextLifecyclePhase::Start).unwrap();
        assert_eq!(start_inv.len(), 2);

        let turn_inv = grok_context_inventory(ContextLifecyclePhase::PerTurn).unwrap();
        assert_eq!(turn_inv.len(), 1);
    }

    #[test]
    fn fake_cli_probe_succeeds_and_reads_models() {
        let fixture = fixture_cli();
        if !fixture.exists() {
            return;
        }

        let executable = GrokExecutable {
            path: fixture,
            version: "1.0.4-e2b819f".into(),
        };

        let profile = probe(&executable, Duration::from_secs(5)).unwrap();
        assert_eq!(profile.version, "1.0.4-e2b819f");
        assert!(profile.load_session);
        assert!(!profile.models.is_empty());
        assert!(profile.models.iter().any(|m| m.id == "grok-code"));
    }

    #[test]
    fn fake_cli_probe_fails_on_terminal_mode() {
        let fixture = fixture_cli();
        if !fixture.exists() {
            return;
        }

        std::env::set_var("BRIDGE_GROK_FAKE_MODE", "terminal");
        let executable = GrokExecutable {
            path: fixture,
            version: "1.0.4".into(),
        };

        let err = probe(&executable, Duration::from_secs(3)).unwrap_err();
        std::env::remove_var("BRIDGE_GROK_FAKE_MODE");

        assert!(matches!(err, GrokUnavailable::NotProtocol { .. }));
    }

    #[test]
    fn fake_cli_probe_fails_on_needs_login() {
        let fixture = fixture_cli();
        if !fixture.exists() {
            return;
        }

        std::env::set_var("BRIDGE_GROK_FAKE_MODE", "needs_login");
        let executable = GrokExecutable {
            path: fixture,
            version: "1.0.4".into(),
        };

        let err = probe(&executable, Duration::from_secs(3)).unwrap_err();
        std::env::remove_var("BRIDGE_GROK_FAKE_MODE");

        assert!(matches!(err, GrokUnavailable::NeedsSignIn { .. }));
    }

    #[test]
    fn classify_probe_failure_modes() {
        let err = classify_probe_failure(
            "1.0.0",
            &AcpError::AuthenticationRequired {
                reason: "auth required".into(),
            },
            &[],
        );
        assert_eq!(err, GrokUnavailable::NeedsSignIn { version: "1.0.0".into() });

        let err2 = classify_probe_failure(
            "1.0.0",
            &AcpError::HandshakeFailed {
                reason: "failed".into(),
                output: Some("ANSI terminal noise".into()),
            },
            &[],
        );
        assert_eq!(
            err2,
            GrokUnavailable::NotProtocol {
                version: "1.0.0".into(),
                output: Some("ANSI terminal noise".into()),
            }
        );
    }

    #[test]
    fn adapter_descriptor_matches_contract() {
        let adapter = GrokAdapter::with_probe(Ok(GrokProfile {
            executable: PathBuf::from("/bin/grok"),
            version: "1.0.4".into(),
            agent_name: Some("Grok Build".into()),
            load_session: true,
            resume_session: false,
            additional_directories: false,
            prompt_images: true,
            auth_methods: vec!["grok_login".into()],
            modes: vec!["agent".into(), "plan".into()],
            current_mode: Some("agent".into()),
            models: vec![ModelOption {
                id: "grok-code".into(),
                label: "Grok Code".into(),
                tier: CapabilityTier::Standard,
                default_for_tier: true,
            }],
            default_model: Some("grok-code".into()),
        }));

        let desc = adapter.descriptor();
        assert_eq!(desc.id, "grok");
        assert_eq!(desc.label, "Grok Build");
        assert!(desc.available);
        assert_eq!(desc.auth_state, AuthState::SignedIn);
        assert_eq!(desc.version.as_deref(), Some("1.0.4"));
        assert_eq!(desc.default_model.as_deref(), Some("grok-code"));
    }

    #[test]
    fn grok_adapter_descriptor_reports_when_uninstalled() {
        let adapter = GrokAdapter::with_probe(Err(GrokUnavailable::NotInstalled));
        let desc = adapter.descriptor();
        assert_eq!(desc.id, "grok");
        assert_eq!(desc.label, "Grok Build");
        assert!(!desc.available);
        assert_eq!(desc.auth_state, AuthState::Unknown);
        assert!(desc.unavailable_reason.is_some());
    }
}

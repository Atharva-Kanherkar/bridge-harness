//! The boundary a supported agent implements, and the driver primitives that
//! carry its transport.
//!
//! **Two halves, deliberately separated.** A [`BackendDriver`] owns transport,
//! framing, and bounded failure reporting for one *shape* of backend — an SDK
//! sidecar, a structured server, an ACP server, a bounded structured CLI. An
//! [`AgentIntegration`] owns what one *agent* means: its descriptor, its model
//! mapping, how its frames normalize, its permission model, its resume
//! semantics. Neither knows the other's job, which is what makes a second agent
//! on an existing shape a new integration rather than a new driver.
//!
//! **Capabilities are evidence, not authority.** A runtime advertises what it
//! can do at handshake. That advertisement is checked *against* the verified
//! profile and never widens it: claiming more than the profile lists is refused
//! as drift, claiming less degrades. The check is a free function rather than a
//! trait method precisely because an integration that could implement it could
//! weaken it.
//!
//! **Nothing here decides a permission.** An integration reports a request; the
//! caller answers it. There is no method on this boundary that returns a
//! decision, and a test enumerates the trait to keep it that way.

use crate::{
    adapters::ShutdownReason,
    agent::NormalizedEvent,
    backend_binding::{BackendCandidate, BackendError, BackendKind, BackendResolver},
    model::CapabilityTier,
    verified_catalog::{Catalog, IntegrationConfig, VerifiedEntry},
};
use bridge_protocol::messages::{AgentId, BackendId};
use serde::Serialize;
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::Arc};

/// Why an integration could not do what was asked. One code per condition, so
/// a caller can act on them differently without matching on prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegrationError {
    /// The transport failed. Carries a bounded reason, never a transcript.
    Transport { reason: String },
    /// This backend shape cannot abandon a turn in flight. Reported rather
    /// than swallowed: a caller that asked for an interrupt is entitled to
    /// know it did not happen.
    InterruptUnsupported { backend: String },
    /// This backend has no native resume, so Bridge must not ask it to.
    ResumeUnsupported { backend: String },
    /// The runtime advertised a capability its verified profile does not list.
    CapabilityDrift {
        agent: String,
        undeclared: Vec<String>,
    },
    /// A vendor-specific method nothing has a typed handler for. Reported,
    /// never dispatched, and never silently dropped.
    UnknownExtensionMethod { backend: String, method: String },
    /// The driver's shape is not the one the verified entry names.
    BackendShapeMismatch { expected: String, found: String },
    /// The runtime process ended. Carries the bounded failure context.
    RuntimeDied { reason: String },
    /// The catalog names a backend this build has no integration for. Not a
    /// broken catalog — an older Bridge reading a newer snapshot.
    NoIntegration { agent: String, backend: String },
    /// Two integrations claim one backend. A build mistake, refused at
    /// registration rather than resolved by whichever registered first.
    DuplicateIntegration { backend: String },
}

impl IntegrationError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Transport { .. } => "integration_transport_failed",
            Self::InterruptUnsupported { .. } => "interrupt_unsupported",
            Self::ResumeUnsupported { .. } => "resume_unsupported",
            Self::CapabilityDrift { .. } => "capability_drift",
            Self::UnknownExtensionMethod { .. } => "unknown_extension_method",
            Self::BackendShapeMismatch { .. } => "backend_shape_mismatch",
            Self::RuntimeDied { .. } => "runtime_died",
            Self::NoIntegration { .. } => "no_integration",
            Self::DuplicateIntegration { .. } => "duplicate_integration",
        }
    }
}

impl std::fmt::Display for IntegrationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport { reason } => {
                write!(formatter, "integration transport failed: {reason}")
            }
            Self::InterruptUnsupported { backend } => write!(
                formatter,
                "{backend} cannot interrupt a turn in flight; the turn is still running"
            ),
            Self::ResumeUnsupported { backend } => {
                write!(formatter, "{backend} has no native resume")
            }
            Self::CapabilityDrift { agent, undeclared } => write!(
                formatter,
                "{agent} advertises {} which its verified profile does not list",
                undeclared.join(", ")
            ),
            Self::UnknownExtensionMethod { backend, method } => write!(
                formatter,
                "{backend} received the extension method {method}, which nothing handles"
            ),
            Self::BackendShapeMismatch { expected, found } => write!(
                formatter,
                "this entry names a {expected} backend but the driver is {found}"
            ),
            Self::RuntimeDied { reason } => write!(formatter, "the runtime ended: {reason}"),
            Self::NoIntegration { agent, backend } => write!(
                formatter,
                "{agent} is served by {backend}, which this build has no integration for"
            ),
            Self::DuplicateIntegration { backend } => {
                write!(formatter, "{backend} already has an integration")
            }
        }
    }
}

impl std::error::Error for IntegrationError {}

/// Whether this backend shape can abandon a turn in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptSupport {
    /// The transport has a way to say "stop".
    Native,
    /// It does not. Killing the process is the only stop, and that is the
    /// caller's decision to make, not one to make quietly on its behalf.
    Unsupported,
}

/// Whether a provider can pick a prior session back up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeSupport {
    /// The provider replays its own session from its own id.
    Native,
    /// It cannot. Bridge must never ask, exactly as `AdapterRegistry` already
    /// refuses for the built-ins that do not resume.
    None,
}

/// How a runtime handles work that needs a user's say-so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionModel {
    /// The runtime asks before acting, and Bridge asks the user.
    RequestsApproval,
    /// The runtime never asks; the sandbox it was launched in is the only
    /// control. Declared so a caller knows no request is coming, rather than
    /// waiting for one that never arrives.
    SandboxOnly,
}

/// What a runtime says about itself at handshake. Evidence, checked against the
/// verified profile — never a way to widen it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Advertisement {
    pub capabilities: Vec<String>,
    /// The vendor-specific methods it says it speaks. Still gated by the
    /// catalog's own list before anything is dispatched.
    pub extension_methods: Vec<String>,
    /// The provider's own session id, when it mints one at handshake.
    pub provider_session_id: Option<String>,
}

/// The result of checking an advertisement against a verified profile.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilityReport {
    /// Present in both. The only capabilities anything may rely on.
    pub agreed: Vec<String>,
    /// In the profile, absent from the runtime. Degraded, not blocked: an
    /// older build that does less is still a usable build.
    pub degraded: Vec<String>,
}

/// Check a runtime's advertisement against the profile Bridge verified.
///
/// A free function on purpose. Making this a trait method would hand each
/// integration the ability to decide how strictly its own runtime is checked,
/// which is the one decision an integration must not have.
pub fn check_capabilities(
    entry: &VerifiedEntry,
    advertised: &Advertisement,
) -> Result<CapabilityReport, IntegrationError> {
    let undeclared: Vec<String> = advertised
        .capabilities
        .iter()
        .filter(|capability| !entry.capabilities.contains(capability))
        .cloned()
        .collect();
    if !undeclared.is_empty() {
        return Err(IntegrationError::CapabilityDrift {
            agent: entry.agent.as_str().to_owned(),
            undeclared,
        });
    }
    let (agreed, degraded) = entry
        .capabilities
        .iter()
        .cloned()
        .partition(|capability| advertised.capabilities.contains(capability));
    Ok(CapabilityReport { agreed, degraded })
}

/// Who an integration is and what it reaches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationDescriptor {
    pub agent: AgentId,
    pub backend: BackendId,
    pub kind: BackendKind,
    pub label: String,
}

/// What a caller asks for when starting a turn-serving runtime. Data only —
/// the driver turns it into frames, and nothing here is a command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRequest {
    pub cwd: String,
    pub tier: CapabilityTier,
    pub instructions: Option<String>,
}

/// The boundary one supported agent implements.
///
/// Everything on it is either a description of the agent or a pure translation
/// of its frames. Nothing on it decides a permission, launches a process, or
/// reports a capability the profile did not already carry.
pub trait AgentIntegration: Send + Sync {
    fn descriptor(&self) -> IntegrationDescriptor;

    /// Which concrete model a requested tier becomes, when the agent has one.
    fn model_for(&self, tier: CapabilityTier) -> Option<String>;

    /// The frames this agent needs at startup, in order. Data the driver
    /// sends; never something it executes.
    fn startup_frames(&self, request: &LaunchRequest) -> Vec<Value>;

    /// One provider frame into normalized events.
    ///
    /// May return nothing for a frame it does not recognize — the session
    /// turns that into an explicit `unknown` event rather than a silence, so
    /// totality is the framework's guarantee and not each integration's
    /// promise.
    fn normalize(&self, frame: &Value) -> Vec<NormalizedEvent>;

    fn permissions(&self) -> PermissionModel;

    /// Whether this agent's runtime can pick a prior session back up.
    ///
    /// Defaults to whatever the backend shape allows, and exists so an
    /// integration can say *no* on a shape that otherwise could — an ACP server
    /// that never implemented `session/load`, say. `AdapterRegistry`'s
    /// `supports_native_resume` already draws this line per adapter rather than
    /// per transport, and the same runtime can be reached over a shape that
    /// resumes without itself being able to.
    fn resume(&self) -> ResumeSupport {
        ResumeSupport::Native
    }

    /// A typed handler for one vendor-specific official method.
    ///
    /// Only ever reached for a method the catalog permits *and* this
    /// integration named. Returning `None` means "named it, cannot handle this
    /// one", which is reported like any other unknown method.
    fn handle_extension(&self, _method: &str, _params: &Value) -> Option<Vec<NormalizedEvent>> {
        None
    }
}

/// The bytes a driver moves, without saying how they got there.
///
/// A real implementation owns a child process; the tests own a script. Both
/// face the same session, which is what makes "one control plane" a fact about
/// the code rather than a claim about it.
pub trait Transport: Send {
    /// What the runtime says about itself. Read exactly once, at launch.
    fn handshake(&mut self) -> Result<Advertisement, IntegrationError>;
    fn send(&mut self, frame: Value) -> Result<(), IntegrationError>;
    /// The next frame, or `None` when the stream has ended.
    fn next_frame(&mut self) -> Option<Result<Value, IntegrationError>>;
    /// Ask the runtime to abandon the turn in flight. Only called when the
    /// driver declares [`InterruptSupport::Native`].
    fn interrupt(&mut self) -> Result<(), IntegrationError>;
    /// Why the runtime died, once it has: bounded, never a full transcript.
    /// Mirrors the `failure_context` contract the built-in adapters meet.
    fn failure_context(&mut self) -> Option<String>;
    fn shutdown(&mut self, reason: ShutdownReason);
}

/// One shape of backend: how a turn becomes a frame, and what the transport
/// can be asked to do.
///
/// A driver owns framing and policy. It never owns agent-specific behaviour —
/// if a driver ever needs to know which agent it is carrying, the split
/// between these two traits has failed.
pub trait BackendDriver: Send + Sync {
    fn kind(&self) -> BackendKind;
    fn interrupt_support(&self) -> InterruptSupport;
    fn resume_support(&self) -> ResumeSupport;
    /// A user turn, framed for this shape.
    fn turn_frame(&self, text: &str) -> Value;
    /// The answer to a permission request. The decision is the caller's; this
    /// only says how to spell it on the wire.
    fn decision_frame(&self, request_id: &Value, decision: &str) -> Value;
    /// The frame that picks a prior provider session back up. Never called for
    /// a driver declaring [`ResumeSupport::None`].
    fn resume_frame(&self, provider_session_id: &str) -> Value;
}

/// A long-lived official SDK, driven through a Bridge sidecar.
#[derive(Debug, Clone, Copy, Default)]
pub struct SdkSidecarDriver;

/// An official structured application server: JSON-RPC over stdio, or an
/// authenticated loopback server.
#[derive(Debug, Clone, Copy, Default)]
pub struct StructuredServerDriver;

/// An official Agent Client Protocol server.
#[derive(Debug, Clone, Copy, Default)]
pub struct AcpDriver;

/// An official headless CLI with structured output, invoked per turn.
#[derive(Debug, Clone, Copy, Default)]
pub struct StructuredCliDriver;

impl BackendDriver for SdkSidecarDriver {
    fn kind(&self) -> BackendKind {
        BackendKind::SdkSidecar
    }
    fn interrupt_support(&self) -> InterruptSupport {
        InterruptSupport::Native
    }
    fn resume_support(&self) -> ResumeSupport {
        ResumeSupport::Native
    }
    fn turn_frame(&self, text: &str) -> Value {
        json!({"type": "user", "message": {"role": "user", "content": text}})
    }
    fn decision_frame(&self, request_id: &Value, decision: &str) -> Value {
        json!({"type": "control_response", "request_id": request_id, "response": decision})
    }
    fn resume_frame(&self, provider_session_id: &str) -> Value {
        json!({"type": "resume", "session_id": provider_session_id})
    }
}

impl BackendDriver for StructuredServerDriver {
    fn kind(&self) -> BackendKind {
        BackendKind::StructuredServer
    }
    fn interrupt_support(&self) -> InterruptSupport {
        InterruptSupport::Native
    }
    fn resume_support(&self) -> ResumeSupport {
        ResumeSupport::Native
    }
    fn turn_frame(&self, text: &str) -> Value {
        json!({"jsonrpc": "2.0", "method": "session/prompt", "params": {"text": text}})
    }
    fn decision_frame(&self, request_id: &Value, decision: &str) -> Value {
        json!({"jsonrpc": "2.0", "id": request_id, "result": {"decision": decision}})
    }
    fn resume_frame(&self, provider_session_id: &str) -> Value {
        json!({
            "jsonrpc": "2.0",
            "method": "session/resume",
            "params": {"sessionId": provider_session_id}
        })
    }
}

impl BackendDriver for AcpDriver {
    fn kind(&self) -> BackendKind {
        BackendKind::Acp
    }
    fn interrupt_support(&self) -> InterruptSupport {
        InterruptSupport::Native
    }
    fn resume_support(&self) -> ResumeSupport {
        ResumeSupport::Native
    }
    fn turn_frame(&self, text: &str) -> Value {
        json!({
            "jsonrpc": "2.0",
            "method": "session/prompt",
            "params": {"prompt": [{"type": "text", "text": text}]}
        })
    }
    fn decision_frame(&self, request_id: &Value, decision: &str) -> Value {
        json!({"jsonrpc": "2.0", "id": request_id, "result": {"outcome": decision}})
    }
    fn resume_frame(&self, provider_session_id: &str) -> Value {
        json!({
            "jsonrpc": "2.0",
            "method": "session/load",
            "params": {"sessionId": provider_session_id}
        })
    }
}

impl BackendDriver for StructuredCliDriver {
    fn kind(&self) -> BackendKind {
        BackendKind::StructuredCli
    }
    /// A CLI invocation has no channel to say "stop" on. Killing it is a
    /// decision the caller makes with the failure in hand, not one this driver
    /// makes quietly by pretending the interrupt landed.
    fn interrupt_support(&self) -> InterruptSupport {
        InterruptSupport::Unsupported
    }
    /// And no session semantics of its own to resume into.
    fn resume_support(&self) -> ResumeSupport {
        ResumeSupport::None
    }
    fn turn_frame(&self, text: &str) -> Value {
        json!({"prompt": text})
    }
    fn decision_frame(&self, request_id: &Value, decision: &str) -> Value {
        json!({"requestId": request_id, "decision": decision})
    }
    fn resume_frame(&self, _provider_session_id: &str) -> Value {
        // Unreachable through the session, which refuses first. Framed anyway
        // rather than panicking, because a driver is data about a shape.
        json!({})
    }
}

/// A running integration: one agent, on one backend shape, over one transport.
///
/// Every backend shape reaches the caller through this one type. There is no
/// per-shape branch above it, which is the whole claim of the framework —
/// `three_backend_shapes_run_through_one_control_plane` is the proof.
pub struct IntegrationSession {
    integration: Arc<dyn AgentIntegration>,
    driver: Box<dyn BackendDriver>,
    transport: Box<dyn Transport>,
    descriptor: IntegrationDescriptor,
    capabilities: CapabilityReport,
    /// The extension methods the *catalog* permits. Bridge-owned configuration
    /// a runtime cannot add to by advertising more.
    permitted_extensions: Vec<String>,
    config: IntegrationConfig,
    provider_session_id: Option<String>,
}

/// Hand-written because the integration, driver, and transport are trait
/// objects. Reports what a caller can act on — who is running, on what shape,
/// and what the capability check concluded — and never the transport's
/// contents.
impl std::fmt::Debug for IntegrationSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IntegrationSession")
            .field("agent", &self.descriptor.agent.as_str())
            .field("backend", &self.descriptor.backend.as_str())
            .field("kind", &self.driver.kind().as_str())
            .field("capabilities", &self.capabilities)
            .field("provider_session_id", &self.provider_session_id)
            .finish()
    }
}

impl IntegrationSession {
    /// Handshake, check the advertisement against the profile, and send the
    /// integration's startup frames.
    ///
    /// The capability check happens before any startup frame is sent: a
    /// runtime that has already drifted should not be asked to do anything.
    pub fn launch(
        integration: Arc<dyn AgentIntegration>,
        driver: Box<dyn BackendDriver>,
        mut transport: Box<dyn Transport>,
        entry: &VerifiedEntry,
        request: &LaunchRequest,
    ) -> Result<Self, IntegrationError> {
        let expected = BackendKind::from(entry.backend_kind);
        if driver.kind() != expected {
            return Err(IntegrationError::BackendShapeMismatch {
                expected: expected.as_str().to_owned(),
                found: driver.kind().as_str().to_owned(),
            });
        }
        let advertisement = transport.handshake()?;
        let capabilities = check_capabilities(entry, &advertisement)?;
        for frame in integration.startup_frames(request) {
            transport.send(frame)?;
        }
        Ok(Self {
            descriptor: integration.descriptor(),
            integration,
            driver,
            transport,
            capabilities,
            permitted_extensions: entry.integration.extension_methods.clone(),
            config: entry.integration.clone(),
            provider_session_id: advertisement.provider_session_id,
        })
    }

    pub fn descriptor(&self) -> &IntegrationDescriptor {
        &self.descriptor
    }

    /// What this runtime and its profile agree on, and what it cannot do.
    pub fn capabilities(&self) -> &CapabilityReport {
        &self.capabilities
    }

    /// The Bridge-owned configuration for this integration.
    ///
    /// `handshake_timeout_ms` is applied by the process transport that owns a
    /// child; the fake transports these tests use answer immediately and have
    /// nothing to time out. Exposed rather than hidden so the one place that
    /// consumes it can read it from here instead of re-deriving it.
    pub fn config(&self) -> &IntegrationConfig {
        &self.config
    }

    pub fn provider_session_id(&self) -> Option<&str> {
        self.provider_session_id.as_deref()
    }

    pub fn send_turn(&mut self, text: &str) -> Result<(), IntegrationError> {
        let frame = self.driver.turn_frame(text);
        self.transport.send(frame)
    }

    /// Answer a permission request with a decision the *caller* made.
    ///
    /// The session has no opinion about the decision and no path that produces
    /// one. It only knows how this backend shape spells an answer.
    pub fn respond(&mut self, request_id: &Value, decision: &str) -> Result<(), IntegrationError> {
        let frame = self.driver.decision_frame(request_id, decision);
        self.transport.send(frame)
    }

    /// Abandon the turn in flight, or say that this shape cannot.
    ///
    /// The one thing this must never do is return `Ok` without the interrupt
    /// having been delivered.
    pub fn interrupt(&mut self) -> Result<(), IntegrationError> {
        match self.driver.interrupt_support() {
            InterruptSupport::Native => self.transport.interrupt(),
            InterruptSupport::Unsupported => Err(IntegrationError::InterruptUnsupported {
                backend: self.descriptor.backend.as_str().to_owned(),
            }),
        }
    }

    /// Pick a prior provider session back up, or refuse because nothing here
    /// can — the same refusal `AdapterRegistry` already makes.
    pub fn resume(&mut self, provider_session_id: &str) -> Result<(), IntegrationError> {
        match self.resume_support() {
            ResumeSupport::Native => {
                let frame = self.driver.resume_frame(provider_session_id);
                self.transport.send(frame)
            }
            ResumeSupport::None => Err(IntegrationError::ResumeUnsupported {
                backend: self.descriptor.backend.as_str().to_owned(),
            }),
        }
    }

    /// The narrower of what the shape allows and what the agent declares.
    ///
    /// Both get a veto and neither gets to grant: a transport with no resume
    /// cannot be talked into one by an integration, and an integration whose
    /// runtime cannot resume is not made able to by running on a shape that
    /// could.
    pub fn resume_support(&self) -> ResumeSupport {
        match (self.driver.resume_support(), self.integration.resume()) {
            (ResumeSupport::Native, ResumeSupport::Native) => ResumeSupport::Native,
            _ => ResumeSupport::None,
        }
    }

    pub fn permissions(&self) -> PermissionModel {
        self.integration.permissions()
    }

    /// Which concrete model a requested tier becomes for this agent.
    ///
    /// Routed through the session rather than read off the integration so the
    /// caller needs one handle, not two — the same reason every other question
    /// about a running agent is answered here.
    pub fn model_for(&self, tier: CapabilityTier) -> Option<String> {
        self.integration.model_for(tier)
    }

    /// Every frame currently available, normalized.
    ///
    /// Total by construction: a frame the integration does not recognize
    /// becomes an `unknown` event carrying the frame, and a transport error
    /// becomes a `runtime.failed` event. Nothing is dropped, because a dropped
    /// frame is indistinguishable from a frame that never arrived.
    pub fn drain(&mut self) -> Vec<NormalizedEvent> {
        let mut events = Vec::new();
        while let Some(frame) = self.transport.next_frame() {
            match frame {
                Ok(frame) => {
                    let normalized = self.integration.normalize(&frame);
                    if normalized.is_empty() {
                        events.push(unknown_event(&frame));
                    } else {
                        events.extend(normalized);
                    }
                }
                Err(error) => events.push(failure_event(&error)),
            }
        }
        events
    }

    /// Dispatch a vendor-specific official method.
    ///
    /// Gated twice, and the gate runs *before* the handler: the catalog must
    /// permit the method, and the integration must have a typed handler for
    /// it. Anything else is reported with its own code — never dispatched, and
    /// never silently dropped.
    pub fn dispatch_extension(
        &self,
        method: &str,
        params: &Value,
    ) -> Result<Vec<NormalizedEvent>, IntegrationError> {
        let unknown = || IntegrationError::UnknownExtensionMethod {
            backend: self.descriptor.backend.as_str().to_owned(),
            method: method.to_owned(),
        };
        if !self.permitted_extensions.iter().any(|name| name == method) {
            return Err(unknown());
        }
        self.integration
            .handle_extension(method, params)
            .ok_or_else(unknown)
    }

    /// Why the runtime died, once it has. Bounded, and the same shape the
    /// built-in adapters already report.
    pub fn failure_context(&mut self) -> Option<String> {
        self.transport.failure_context()
    }

    pub fn shutdown(&mut self, reason: ShutdownReason) {
        self.transport.shutdown(reason);
    }
}

/// The driver for one backend shape.
///
/// Total over [`BackendKind`], so a new shape must come past this match and be
/// given a driver rather than silently having none.
pub fn driver_for(kind: BackendKind) -> Box<dyn BackendDriver> {
    match kind {
        BackendKind::SdkSidecar => Box::new(SdkSidecarDriver),
        BackendKind::StructuredServer => Box::new(StructuredServerDriver),
        BackendKind::Acp => Box::new(AcpDriver),
        BackendKind::StructuredCli => Box::new(StructuredCliDriver),
    }
}

/// Every integration compiled into this build, keyed by the backend it serves.
///
/// The other half of the "adding an agent is a profile plus an integration"
/// claim: a catalog entry says *what* to run and this says *how*, and neither
/// the orchestrator, the router, the session store, nor the delegation pipeline
/// has to learn a name for it.
#[derive(Default)]
pub struct IntegrationRegistry {
    integrations: BTreeMap<BackendId, Arc<dyn AgentIntegration>>,
}

impl std::fmt::Debug for IntegrationRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IntegrationRegistry")
            .field(
                "backends",
                &self
                    .integrations
                    .keys()
                    .map(BackendId::as_str)
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// What a catalog contributed to a resolver, and what it could not.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogRegistration {
    pub registered: Vec<AgentId>,
    /// Entries this build cannot serve. Reported rather than dropped so a UI
    /// can say "needs a newer Bridge" instead of showing nothing and leaving
    /// the user to guess.
    pub skipped: Vec<SkippedEntry>,
}

/// One catalog entry this build did not register, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedEntry {
    pub agent: AgentId,
    /// A stable code: `no_integration`, `unsupported_platform`, or
    /// `backend_conflict`.
    pub reason: &'static str,
}

impl IntegrationRegistry {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Add one integration. A second claiming the same backend is refused:
    /// two things serving one backend is a build mistake, and picking one
    /// quietly would make which one wins depend on registration order.
    pub fn register(
        &mut self,
        integration: Arc<dyn AgentIntegration>,
    ) -> Result<(), IntegrationError> {
        let backend = integration.descriptor().backend;
        if self.integrations.contains_key(&backend) {
            return Err(IntegrationError::DuplicateIntegration {
                backend: backend.as_str().to_owned(),
            });
        }
        self.integrations.insert(backend, integration);
        Ok(())
    }

    pub fn get(&self, backend: &BackendId) -> Option<&Arc<dyn AgentIntegration>> {
        self.integrations.get(backend)
    }

    /// Whether this build can run this entry here: it has an integration for
    /// the backend, and the entry supports this platform.
    pub fn can_serve(&self, entry: &VerifiedEntry) -> bool {
        self.integrations.contains_key(&entry.backend) && entry.installable_here()
    }

    /// Launch a verified entry: the integration by backend, the driver by
    /// shape, and the session over both.
    pub fn launch(
        &self,
        entry: &VerifiedEntry,
        transport: Box<dyn Transport>,
        request: &LaunchRequest,
    ) -> Result<IntegrationSession, IntegrationError> {
        let integration =
            self.get(&entry.backend)
                .cloned()
                .ok_or_else(|| IntegrationError::NoIntegration {
                    agent: entry.agent.as_str().to_owned(),
                    backend: entry.backend.as_str().to_owned(),
                })?;
        IntegrationSession::launch(
            integration,
            driver_for(BackendKind::from(entry.backend_kind)),
            transport,
            entry,
            request,
        )
    }

    /// Offer every entry this build can serve to #163's resolver.
    ///
    /// An entry with no compiled-in integration is *skipped*, not refused. A
    /// snapshot naming an agent that a newer Bridge supports is an ordinary
    /// thing for an older build to receive, and rejecting the catalog over it
    /// would make every future agent a breaking change for every build that
    /// predates it. This is also why "the build can reach this backend" is a
    /// question asked here rather than during snapshot validation: the same
    /// document is valid on the build that ships the integration and on the one
    /// that does not.
    ///
    /// Infallible on purpose. A snapshot arrives from the network, and every
    /// failure mode here — an unknown backend, a wrong platform, a name that
    /// collides with a built-in — is a reason to skip one entry, never a reason
    /// to fail the call that boots the app. A remote document must not be able
    /// to leave Bridge unable to resolve the agents it already had.
    pub fn offer_catalog(
        &self,
        catalog: &Catalog,
        resolver: &mut BackendResolver,
    ) -> CatalogRegistration {
        let mut registration = CatalogRegistration::default();
        for entry in catalog.entries() {
            let reason = if !self.integrations.contains_key(&entry.backend) {
                Some("no_integration")
            } else if !entry.installable_here() {
                Some("unsupported_platform")
            } else {
                None
            };
            let outcome = match reason {
                Some(reason) => Err(reason),
                None => resolver
                    .register(
                        &entry.agent,
                        BackendCandidate {
                            backend: entry.backend.clone(),
                            // For a catalog agent the executor is this registry,
                            // which is keyed by backend — so the backend id *is*
                            // the executor key, rather than a second name to
                            // keep in step with it.
                            adapter_id: entry.backend.as_str().to_owned(),
                            kind: BackendKind::from(entry.backend_kind),
                        },
                    )
                    .map_err(|_: BackendError| "backend_conflict"),
            };
            match outcome {
                Ok(()) => registration.registered.push(entry.agent.clone()),
                Err(reason) => registration.skipped.push(SkippedEntry {
                    agent: entry.agent.clone(),
                    reason,
                }),
            }
        }
        registration
    }
}

/// A frame nothing recognized, reported rather than dropped.
fn unknown_event(frame: &Value) -> NormalizedEvent {
    let mut event = NormalizedEvent::new("provider.unknown");
    event.data = json!({"frame": frame});
    event
}

fn failure_event(error: &IntegrationError) -> NormalizedEvent {
    let mut event = NormalizedEvent::new("runtime.failed");
    event.status = Some("failed".into());
    event.text = Some(error.to_string());
    event.data = json!({"code": error.code()});
    event
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verified_catalog::{
        CatalogBackendKind, CatalogRecipe, VendorRequirement, Verification, VerificationStatus,
    };
    use bridge_protocol::messages::BackendVersion;
    use std::{collections::BTreeSet, sync::Mutex};

    /// A verified entry for a fake agent on one backend shape.
    fn verified_entry(
        agent: &str,
        kind: CatalogBackendKind,
        capabilities: &[&str],
    ) -> VerifiedEntry {
        VerifiedEntry {
            agent: AgentId::parse(agent).unwrap(),
            label: "Fake".into(),
            vendor: "Fake Inc".into(),
            license: "Apache-2.0".into(),
            source_url: "https://example.test/fake".into(),
            version: BackendVersion::parse("1.0.0").unwrap(),
            platforms: vec![],
            recipe: CatalogRecipe::ReleaseArtifact {
                url: "https://example.test/fake.tar.gz".into(),
                sha256: "0".repeat(64),
                archive: crate::verified_catalog::ArchiveKind::TarGz,
                entrypoint: "bin/fake".into(),
            },
            // Overwritten below for the tests that need this entry to be
            // installable on the machine running them.
            backend: BackendId::parse(&format!("{agent}.backend")).unwrap(),
            backend_kind: kind,
            integration: IntegrationConfig {
                handshake_timeout_ms: Some(5_000),
                multiplexes_sessions: false,
                extension_methods: vec!["fake/officialThing".into()],
            },
            vendor_requirement: VendorRequirement::None,
            capabilities: capabilities
                .iter()
                .map(|name| (*name).to_string())
                .collect(),
            blocked_versions: vec![],
            minimum_bridge_version: "0.1.0".into(),
            verification: Verification {
                status: VerificationStatus::Verified,
                suite_version: 1,
                verified_at: "2026-08-15T00:00:00Z".into(),
                evidence_ref: "evidence-0001".into(),
            },
        }
    }

    /// A transport driven by a script rather than a process. Records what was
    /// sent, so a test can assert that nothing was sent — which is how
    /// "reports rather than decides" is checked.
    #[derive(Default)]
    struct ScriptedTransport {
        advertisement: Advertisement,
        frames: Vec<Result<Value, IntegrationError>>,
        sent: Arc<Mutex<Vec<Value>>>,
        interrupts: Arc<Mutex<usize>>,
        interrupt_fails: bool,
        failure: Option<String>,
        shutdowns: Arc<Mutex<Vec<ShutdownReason>>>,
    }

    impl Transport for ScriptedTransport {
        fn handshake(&mut self) -> Result<Advertisement, IntegrationError> {
            Ok(self.advertisement.clone())
        }
        fn send(&mut self, frame: Value) -> Result<(), IntegrationError> {
            self.sent.lock().unwrap().push(frame);
            Ok(())
        }
        fn next_frame(&mut self) -> Option<Result<Value, IntegrationError>> {
            if self.frames.is_empty() {
                return None;
            }
            Some(self.frames.remove(0))
        }
        fn interrupt(&mut self) -> Result<(), IntegrationError> {
            if self.interrupt_fails {
                return Err(IntegrationError::Transport {
                    reason: "the runtime did not acknowledge the interrupt".into(),
                });
            }
            *self.interrupts.lock().unwrap() += 1;
            Ok(())
        }
        fn failure_context(&mut self) -> Option<String> {
            self.failure.clone()
        }
        fn shutdown(&mut self, reason: ShutdownReason) {
            self.shutdowns.lock().unwrap().push(reason);
        }
    }

    /// One fake agent. The same integration behind every backend shape, so a
    /// test that runs three shapes through the control plane is testing the
    /// control plane rather than three different agents.
    struct FakeIntegration {
        agent: String,
        kind: BackendKind,
        permissions: PermissionModel,
        resume: ResumeSupport,
        extension_calls: Arc<Mutex<Vec<String>>>,
    }

    impl AgentIntegration for FakeIntegration {
        fn descriptor(&self) -> IntegrationDescriptor {
            IntegrationDescriptor {
                agent: AgentId::parse(&self.agent).unwrap(),
                backend: BackendId::parse(&format!("{}.backend", self.agent)).unwrap(),
                kind: self.kind,
                label: "Fake".into(),
            }
        }
        fn model_for(&self, tier: CapabilityTier) -> Option<String> {
            Some(format!("fake-{}", tier.as_str()))
        }
        fn startup_frames(&self, request: &LaunchRequest) -> Vec<Value> {
            vec![json!({"type": "start", "cwd": request.cwd})]
        }
        fn normalize(&self, frame: &Value) -> Vec<NormalizedEvent> {
            match frame.get("type").and_then(Value::as_str) {
                Some("text") => {
                    let mut event = NormalizedEvent::new("message.completed");
                    event.role = Some("assistant".into());
                    event.text = frame.get("text").and_then(Value::as_str).map(str::to_owned);
                    vec![event]
                }
                Some("permission") => {
                    let mut event = NormalizedEvent::new("approval.requested");
                    event.data =
                        json!({"requestId": frame.get("id").cloned().unwrap_or(json!(null))});
                    vec![event]
                }
                _ => vec![],
            }
        }
        fn permissions(&self) -> PermissionModel {
            self.permissions
        }
        fn resume(&self) -> ResumeSupport {
            self.resume
        }
        fn handle_extension(&self, method: &str, _params: &Value) -> Option<Vec<NormalizedEvent>> {
            self.extension_calls.lock().unwrap().push(method.to_owned());
            (method == "fake/officialThing")
                .then(|| vec![NormalizedEvent::new("extension.handled")])
        }
    }

    fn integration(agent: &str, kind: BackendKind) -> Arc<FakeIntegration> {
        Arc::new(FakeIntegration {
            agent: agent.into(),
            kind,
            permissions: PermissionModel::RequestsApproval,
            resume: ResumeSupport::Native,
            extension_calls: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// The same fake, on a shape that resumes, declaring that its own runtime
    /// cannot.
    fn integration_without_resume(agent: &str, kind: BackendKind) -> Arc<FakeIntegration> {
        Arc::new(FakeIntegration {
            agent: agent.into(),
            kind,
            permissions: PermissionModel::RequestsApproval,
            resume: ResumeSupport::None,
            extension_calls: Arc::new(Mutex::new(Vec::new())),
        })
    }

    fn driver_for(kind: BackendKind) -> Box<dyn BackendDriver> {
        match kind {
            BackendKind::SdkSidecar => Box::new(SdkSidecarDriver),
            BackendKind::StructuredServer => Box::new(StructuredServerDriver),
            BackendKind::Acp => Box::new(AcpDriver),
            BackendKind::StructuredCli => Box::new(StructuredCliDriver),
        }
    }

    fn catalog_kind(kind: BackendKind) -> CatalogBackendKind {
        match kind {
            BackendKind::SdkSidecar => CatalogBackendKind::SdkSidecar,
            BackendKind::StructuredServer => CatalogBackendKind::StructuredServer,
            BackendKind::Acp => CatalogBackendKind::Acp,
            BackendKind::StructuredCli => CatalogBackendKind::StructuredCli,
        }
    }

    fn request() -> LaunchRequest {
        LaunchRequest {
            cwd: "/tmp/fake".into(),
            tier: CapabilityTier::Standard,
            instructions: None,
        }
    }

    #[test]
    fn three_backend_shapes_run_through_one_control_plane() {
        // SDK sidecar, structured server, and ACP. The loop body is the
        // assertion: there is no per-shape branch in it, because the caller
        // never needs one.
        for kind in [
            BackendKind::SdkSidecar,
            BackendKind::StructuredServer,
            BackendKind::Acp,
        ] {
            let agent = format!("fake-{}", kind.as_str());
            let entry = verified_entry(&agent, catalog_kind(kind), &["messages", "streaming"]);
            let sent = Arc::new(Mutex::new(Vec::new()));
            let shutdowns = Arc::new(Mutex::new(Vec::new()));
            let transport = ScriptedTransport {
                advertisement: Advertisement {
                    capabilities: vec!["messages".into(), "streaming".into()],
                    extension_methods: vec![],
                    provider_session_id: Some("provider-1".into()),
                },
                frames: vec![Ok(json!({"type": "text", "text": "hello"}))],
                sent: sent.clone(),
                shutdowns: shutdowns.clone(),
                ..Default::default()
            };

            let mut session = IntegrationSession::launch(
                integration(&agent, kind),
                driver_for(kind),
                Box::new(transport),
                &entry,
                &request(),
            )
            .unwrap();

            assert_eq!(session.provider_session_id(), Some("provider-1"));
            assert!(session.capabilities().degraded.is_empty());
            assert_eq!(
                session.model_for(CapabilityTier::Strong).as_deref(),
                Some("fake-strong"),
                "the tier mapping is the integration's, reached through the session"
            );

            session.send_turn("do the thing").unwrap();
            let events = session.drain();
            assert_eq!(events.len(), 1, "{kind:?}");
            assert_eq!(events[0].kind, "message.completed");
            assert_eq!(events[0].text.as_deref(), Some("hello"));

            session.interrupt().unwrap();
            session.resume("provider-1").unwrap();
            session.shutdown(ShutdownReason::Completed);

            // The startup frame, the turn, and the resume — three sends, in
            // this shape's own framing.
            assert_eq!(sent.lock().unwrap().len(), 3, "{kind:?}");
            assert_eq!(
                shutdowns.lock().unwrap().as_slice(),
                [ShutdownReason::Completed]
            );
        }
    }

    #[test]
    fn an_integration_reports_permissions_rather_than_deciding_them() {
        let entry = verified_entry("fake", CatalogBackendKind::Acp, &["messages"]);
        let sent = Arc::new(Mutex::new(Vec::new()));
        let transport = ScriptedTransport {
            advertisement: Advertisement {
                capabilities: vec!["messages".into()],
                ..Default::default()
            },
            frames: vec![Ok(json!({"type": "permission", "id": 7}))],
            sent: sent.clone(),
            ..Default::default()
        };
        let mut session = IntegrationSession::launch(
            integration("fake", BackendKind::Acp),
            Box::new(AcpDriver),
            Box::new(transport),
            &entry,
            &request(),
        )
        .unwrap();
        assert_eq!(session.permissions(), PermissionModel::RequestsApproval);

        let events = session.drain();
        assert_eq!(events[0].kind, "approval.requested");
        // Draining a permission request sends nothing. The only frame on the
        // wire is the startup one: no self-approval happened on the way past.
        assert_eq!(
            sent.lock().unwrap().len(),
            1,
            "surfacing a permission request must not answer it"
        );

        // The caller decides, and only then does an answer go out.
        session.respond(&json!(7), "allow").unwrap();
        let frames = sent.lock().unwrap();
        assert_eq!(frames[1].pointer("/result/outcome").unwrap(), "allow");
    }

    #[test]
    fn an_interrupt_is_delivered_or_reported_as_unsupported() {
        // A shape that can interrupt does.
        let entry = verified_entry("fake", CatalogBackendKind::Acp, &["messages"]);
        let interrupts = Arc::new(Mutex::new(0));
        let mut session = IntegrationSession::launch(
            integration("fake", BackendKind::Acp),
            Box::new(AcpDriver),
            Box::new(ScriptedTransport {
                advertisement: Advertisement {
                    capabilities: vec!["messages".into()],
                    ..Default::default()
                },
                interrupts: interrupts.clone(),
                ..Default::default()
            }),
            &entry,
            &request(),
        )
        .unwrap();
        session.interrupt().unwrap();
        assert_eq!(*interrupts.lock().unwrap(), 1);

        // A CLI cannot, and says so rather than returning Ok over a turn that
        // is still running.
        let entry = verified_entry("cli", CatalogBackendKind::StructuredCli, &["messages"]);
        let mut session = IntegrationSession::launch(
            integration("cli", BackendKind::StructuredCli),
            Box::new(StructuredCliDriver),
            Box::new(ScriptedTransport {
                advertisement: Advertisement {
                    capabilities: vec!["messages".into()],
                    ..Default::default()
                },
                ..Default::default()
            }),
            &entry,
            &request(),
        )
        .unwrap();
        let error = session.interrupt().unwrap_err();
        assert_eq!(error.code(), "interrupt_unsupported");

        // And a transport that fails to deliver one reports the failure rather
        // than reporting success.
        let entry = verified_entry("fake", CatalogBackendKind::Acp, &["messages"]);
        let mut session = IntegrationSession::launch(
            integration("fake", BackendKind::Acp),
            Box::new(AcpDriver),
            Box::new(ScriptedTransport {
                advertisement: Advertisement {
                    capabilities: vec!["messages".into()],
                    ..Default::default()
                },
                interrupt_fails: true,
                ..Default::default()
            }),
            &entry,
            &request(),
        )
        .unwrap();
        assert_eq!(
            session.interrupt().unwrap_err().code(),
            "integration_transport_failed"
        );
    }

    #[test]
    fn resume_semantics_are_declared_and_honoured() {
        let entry = verified_entry("cli", CatalogBackendKind::StructuredCli, &["messages"]);
        let sent = Arc::new(Mutex::new(Vec::new()));
        let mut session = IntegrationSession::launch(
            integration("cli", BackendKind::StructuredCli),
            Box::new(StructuredCliDriver),
            Box::new(ScriptedTransport {
                advertisement: Advertisement {
                    capabilities: vec!["messages".into()],
                    ..Default::default()
                },
                sent: sent.clone(),
                ..Default::default()
            }),
            &entry,
            &request(),
        )
        .unwrap();

        assert_eq!(session.resume_support(), ResumeSupport::None);
        let error = session.resume("provider-1").unwrap_err();
        assert_eq!(error.code(), "resume_unsupported");
        // Declared and honoured: refusing means nothing went to the runtime,
        // so it was never asked to do something it cannot do.
        assert_eq!(sent.lock().unwrap().len(), 1, "only the startup frame");

        // The other direction: a shape that resumes, carrying an agent whose
        // own runtime does not. Both get a veto, so this refuses too — the
        // distinction AdapterRegistry already draws per adapter rather than
        // per transport.
        let entry = verified_entry("fake", CatalogBackendKind::Acp, &["messages"]);
        let sent = Arc::new(Mutex::new(Vec::new()));
        let mut session = IntegrationSession::launch(
            integration_without_resume("fake", BackendKind::Acp),
            Box::new(AcpDriver),
            Box::new(ScriptedTransport {
                advertisement: Advertisement {
                    capabilities: vec!["messages".into()],
                    ..Default::default()
                },
                sent: sent.clone(),
                ..Default::default()
            }),
            &entry,
            &request(),
        )
        .unwrap();
        assert_eq!(
            AcpDriver.resume_support(),
            ResumeSupport::Native,
            "the shape itself can resume"
        );
        assert_eq!(
            session.resume_support(),
            ResumeSupport::None,
            "but the agent declared it cannot, and the narrower answer wins"
        );
        assert_eq!(
            session.resume("provider-1").unwrap_err().code(),
            "resume_unsupported"
        );
        assert_eq!(sent.lock().unwrap().len(), 1, "only the startup frame");
    }

    #[test]
    fn a_fake_integrations_events_are_shaped_like_a_built_in_adapters() {
        // The whole point of normalizing: a marketplace agent's events must be
        // indistinguishable in shape from the three Bridge already ships, so
        // nothing downstream needs to know which kind of agent produced them.
        //
        // The vocabulary is read out of `agent.rs` rather than restated here —
        // a hand-copied list would drift from the normalizers the moment one
        // gains a kind, and drift is exactly what this is checking for.
        let normalizers = include_str!("agent.rs");
        let built_in_kinds: BTreeSet<&str> = normalizers
            .match_indices("with_data(\"")
            .chain(normalizers.match_indices("NormalizedEvent::new(\""))
            .filter_map(|(index, needle)| {
                normalizers[index + needle.len()..]
                    .split('"')
                    .next()
                    .filter(|kind| !kind.is_empty())
            })
            .collect();
        assert!(
            built_in_kinds.len() > 10,
            "expected the built-in event vocabulary, found {built_in_kinds:?}"
        );

        let entry = verified_entry("fake", CatalogBackendKind::Acp, &["messages"]);
        let mut session = IntegrationSession::launch(
            integration("fake", BackendKind::Acp),
            Box::new(AcpDriver),
            Box::new(ScriptedTransport {
                advertisement: Advertisement {
                    capabilities: vec!["messages".into()],
                    ..Default::default()
                },
                frames: vec![
                    Ok(json!({"type": "text", "text": "hello"})),
                    Ok(json!({"type": "permission", "id": 4})),
                    Ok(json!({"type": "somethingNew"})),
                ],
                ..Default::default()
            }),
            &entry,
            &request(),
        )
        .unwrap();
        session.send_turn("do the thing").unwrap();

        let events = session.drain();
        assert_eq!(events.len(), 3);
        for event in &events {
            event
                .validate()
                .unwrap_or_else(|error| panic!("{}: {error}", event.kind));
            assert!(
                built_in_kinds.contains(event.kind.as_str()),
                "{:?} is not a kind a built-in adapter produces; a marketplace \
                 agent must not invent its own vocabulary",
                event.kind
            );
        }
        // Including the one for a frame nothing recognized: the built-ins
        // already report provider.unknown, so an integration's unknown frame
        // arrives looking like theirs rather than like a new kind of problem.
        assert_eq!(events[2].kind, "provider.unknown");
    }

    #[test]
    fn a_dead_process_reports_why_it_died() {
        let entry = verified_entry("fake", CatalogBackendKind::Acp, &["messages"]);
        let mut session = IntegrationSession::launch(
            integration("fake", BackendKind::Acp),
            Box::new(AcpDriver),
            Box::new(ScriptedTransport {
                advertisement: Advertisement {
                    capabilities: vec!["messages".into()],
                    ..Default::default()
                },
                frames: vec![Err(IntegrationError::RuntimeDied {
                    reason: "exit status 1: could not open the workspace".into(),
                })],
                failure: Some("exit status 1: could not open the workspace".into()),
                ..Default::default()
            }),
            &entry,
            &request(),
        )
        .unwrap();

        // The death arrives as an event rather than as a silence.
        let events = session.drain();
        assert_eq!(events[0].kind, "runtime.failed");
        assert_eq!(events[0].data["code"], "runtime_died");
        assert!(events[0].text.as_ref().unwrap().contains("exit status 1"));

        // And the bounded context is available to the supervisor, matching
        // what the built-in adapters already report.
        assert!(session
            .failure_context()
            .unwrap()
            .contains("could not open the workspace"));
    }

    #[test]
    fn normalization_is_total_over_a_fixture_stream() {
        // Every frame in the stream produces an event. The two the integration
        // knows normalize; the two it does not are reported as unknown rather
        // than dropped, because a dropped frame and a frame that never arrived
        // look identical downstream.
        let entry = verified_entry("fake", CatalogBackendKind::Acp, &["messages"]);
        let stream = [
            json!({"type": "text", "text": "one"}),
            json!({"type": "somethingNew", "payload": 1}),
            json!({"type": "permission", "id": 3}),
            json!({"unrecognizable": true}),
        ];
        let mut session = IntegrationSession::launch(
            integration("fake", BackendKind::Acp),
            Box::new(AcpDriver),
            Box::new(ScriptedTransport {
                advertisement: Advertisement {
                    capabilities: vec!["messages".into()],
                    ..Default::default()
                },
                frames: stream.iter().cloned().map(Ok).collect(),
                ..Default::default()
            }),
            &entry,
            &request(),
        )
        .unwrap();

        let events = session.drain();
        assert_eq!(events.len(), stream.len(), "no frame may be dropped");
        assert_eq!(events[1].kind, "provider.unknown");
        assert_eq!(events[1].data["frame"], stream[1]);
        assert_eq!(events[3].kind, "provider.unknown");
        for event in &events {
            event
                .validate()
                .expect("every normalized event must be valid");
        }
    }

    #[test]
    fn advertised_capabilities_beyond_the_profile_are_refused_as_drift() {
        // A runtime claiming something its verified profile does not list.
        // Refusing is the point: a capability is evidence to check, never
        // authority to unlock.
        let entry = verified_entry("fake", CatalogBackendKind::Acp, &["messages"]);
        let error = IntegrationSession::launch(
            integration("fake", BackendKind::Acp),
            Box::new(AcpDriver),
            Box::new(ScriptedTransport {
                advertisement: Advertisement {
                    capabilities: vec!["messages".into(), "runCommands".into()],
                    ..Default::default()
                },
                ..Default::default()
            }),
            &entry,
            &request(),
        )
        .unwrap_err();
        assert_eq!(error.code(), "capability_drift");
        assert!(error.to_string().contains("runCommands"), "{error}");

        // Claiming fewer degrades instead: an older build that does less is
        // still a usable build.
        let entry = verified_entry("fake", CatalogBackendKind::Acp, &["messages", "streaming"]);
        let session = IntegrationSession::launch(
            integration("fake", BackendKind::Acp),
            Box::new(AcpDriver),
            Box::new(ScriptedTransport {
                advertisement: Advertisement {
                    capabilities: vec!["messages".into()],
                    ..Default::default()
                },
                ..Default::default()
            }),
            &entry,
            &request(),
        )
        .unwrap();
        assert_eq!(session.capabilities().agreed, ["messages"]);
        assert_eq!(session.capabilities().degraded, ["streaming"]);
    }

    #[test]
    fn a_drifting_runtime_is_refused_before_it_is_asked_to_do_anything() {
        // The ordering claim behind the drift check: a runtime that has
        // already drifted never sees a startup frame.
        let entry = verified_entry("fake", CatalogBackendKind::Acp, &["messages"]);
        let sent = Arc::new(Mutex::new(Vec::new()));
        let error = IntegrationSession::launch(
            integration("fake", BackendKind::Acp),
            Box::new(AcpDriver),
            Box::new(ScriptedTransport {
                advertisement: Advertisement {
                    capabilities: vec!["runCommands".into()],
                    ..Default::default()
                },
                sent: sent.clone(),
                ..Default::default()
            }),
            &entry,
            &request(),
        )
        .unwrap_err();
        assert_eq!(error.code(), "capability_drift");
        assert!(sent.lock().unwrap().is_empty());
    }

    #[test]
    fn an_unknown_extension_method_is_reported_not_dispatched() {
        let entry = verified_entry("fake", CatalogBackendKind::Acp, &["messages"]);
        let integration = integration("fake", BackendKind::Acp);
        let calls = integration.extension_calls.clone();
        let session = IntegrationSession::launch(
            integration,
            Box::new(AcpDriver),
            Box::new(ScriptedTransport {
                advertisement: Advertisement {
                    capabilities: vec!["messages".into()],
                    ..Default::default()
                },
                ..Default::default()
            }),
            &entry,
            &request(),
        )
        .unwrap();

        // The method the catalog permits and the integration handles.
        let handled = session
            .dispatch_extension("fake/officialThing", &json!({}))
            .unwrap();
        assert_eq!(handled[0].kind, "extension.handled");

        // One the catalog does not permit. Reported — and the handler was
        // never reached, which is the difference between "reported" and
        // "dispatched and then rejected".
        let error = session
            .dispatch_extension("fake/somethingElse", &json!({}))
            .unwrap_err();
        assert_eq!(error.code(), "unknown_extension_method");
        assert!(error.to_string().contains("fake/somethingElse"), "{error}");
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            ["fake/officialThing"],
            "an unpermitted method must not reach the handler at all"
        );
    }

    #[test]
    fn a_driver_whose_shape_is_not_the_entrys_is_refused() {
        // The catalog says ACP; the driver is a CLI. Launching anyway would
        // mean the verified profile described something other than what ran.
        let entry = verified_entry("fake", CatalogBackendKind::Acp, &["messages"]);
        let error = IntegrationSession::launch(
            integration("fake", BackendKind::StructuredCli),
            Box::new(StructuredCliDriver),
            Box::new(ScriptedTransport::default()),
            &entry,
            &request(),
        )
        .unwrap_err();
        assert_eq!(error.code(), "backend_shape_mismatch");
    }

    #[test]
    fn every_refusal_condition_has_its_own_code() {
        let codes = [
            IntegrationError::Transport { reason: "r".into() },
            IntegrationError::InterruptUnsupported {
                backend: "b".into(),
            },
            IntegrationError::ResumeUnsupported {
                backend: "b".into(),
            },
            IntegrationError::CapabilityDrift {
                agent: "a".into(),
                undeclared: vec![],
            },
            IntegrationError::UnknownExtensionMethod {
                backend: "b".into(),
                method: "m".into(),
            },
            IntegrationError::BackendShapeMismatch {
                expected: "e".into(),
                found: "f".into(),
            },
            IntegrationError::RuntimeDied { reason: "r".into() },
            IntegrationError::NoIntegration {
                agent: "a".into(),
                backend: "b".into(),
            },
            IntegrationError::DuplicateIntegration {
                backend: "b".into(),
            },
        ]
        .map(|error| error.code());
        let mut unique = codes.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            codes.len(),
            "codes must be distinct: {codes:?}"
        );
    }

    /// The same entry, supporting the platform the test is running on, so
    /// `installable_here` is true and the seam can be exercised end to end.
    fn local_entry(agent: &str, kind: CatalogBackendKind) -> VerifiedEntry {
        let mut entry = verified_entry(agent, kind, &["messages"]);
        entry.platforms = vec![crate::acp_registry::PlatformTarget::current()
            .expect("these tests need a platform the catalog can name")];
        entry
    }

    /// A signed catalog carrying these entries, installed the way a real one
    /// is. Hand-building a `Catalog` would prove less: the seam this test is
    /// about starts at a document that verified.
    fn catalog_of(entries: Vec<VerifiedEntry>) -> Catalog {
        use ed25519_dalek::{Signer, SigningKey};
        let key = SigningKey::from_bytes(&[9u8; 32]);
        let trust = crate::verified_catalog::TrustRoot::from_keys([(
            "seam-key".to_owned(),
            key.verifying_key().to_bytes(),
        )])
        .unwrap();
        let document = serde_json::to_string(&json!({
            "schemaVersion": 1,
            "generation": 2,
            "publishedAt": "2026-08-15T00:00:00Z",
            "minimumBridgeVersion": "0.1.0",
            "entries": entries,
        }))
        .unwrap();
        let signature = key.sign(document.as_bytes()).to_bytes();
        Catalog::bundled("0.1.0")
            .unwrap()
            .install_snapshot(
                crate::verified_catalog::SignedSnapshot {
                    document: document.as_bytes(),
                    key_id: "seam-key",
                    signature: &signature,
                },
                &trust,
                "0.1.0",
            )
            .expect("the seam fixture must be an installable snapshot")
    }

    #[test]
    fn adding_an_agent_touches_only_a_profile_and_an_integration() {
        // A profile and an integration, and nothing else. No orchestrator
        // change, no router change, no session store change, no delegation
        // change — the functional half first, the structural half below.
        let entry = local_entry("newcomer", CatalogBackendKind::Acp);
        let catalog = catalog_of(vec![entry.clone()]);

        let mut registry = IntegrationRegistry::empty();
        registry
            .register(integration("newcomer", BackendKind::Acp))
            .unwrap();

        // It reaches #163's resolver as an ordinary candidate.
        let mut resolver = BackendResolver::empty();
        let registration = registry.offer_catalog(&catalog, &mut resolver);
        assert_eq!(registration.registered, std::slice::from_ref(&entry.agent));
        assert!(registration.skipped.is_empty());

        // And resolves, binds, and resumes through that binding.
        let binding = resolver
            .resolve(&entry.agent, &Default::default())
            .expect("a catalog agent resolves like any other");
        assert_eq!(binding.backend, entry.backend);
        let candidate = resolver
            .resolve_bound(&binding)
            .expect("the bound backend is reachable");
        assert_eq!(candidate.kind, BackendKind::Acp);

        let sent = Arc::new(Mutex::new(Vec::new()));
        let mut session = registry
            .launch(
                &entry,
                Box::new(ScriptedTransport {
                    advertisement: Advertisement {
                        capabilities: vec!["messages".into()],
                        provider_session_id: Some("provider-9".into()),
                        ..Default::default()
                    },
                    sent: sent.clone(),
                    ..Default::default()
                }),
                &request(),
            )
            .unwrap();
        session
            .resume(session.provider_session_id().unwrap().to_owned().as_str())
            .expect("an ACP backend resumes natively");
        assert_eq!(
            sent.lock().unwrap()[1]
                .pointer("/params/sessionId")
                .unwrap(),
            "provider-9"
        );

        // The structural half. These four modules are the ones #166 promises
        // not to touch, and the promise is worth only as much as this check.
        for module in [
            "orchestrator.rs",
            "routing_policy.rs",
            "store.rs",
            "delegation.rs",
        ] {
            let source =
                std::fs::read_to_string(format!("{}/src/{module}", env!("CARGO_MANIFEST_DIR")))
                    .unwrap_or_else(|error| panic!("{module} must be readable: {error}"));
            for forbidden in [
                "agent_integration",
                "IntegrationRegistry",
                "verified_catalog",
                "VerifiedEntry",
            ] {
                assert!(
                    !source.contains(forbidden),
                    "{module} mentions {forbidden:?}: adding an agent must not \
                     require changing it"
                );
            }
        }
    }

    #[test]
    fn an_entry_this_build_cannot_serve_is_skipped_rather_than_refused() {
        // An older Bridge reading a newer snapshot. Rejecting the catalog over
        // an agent it does not carry would make every future agent a breaking
        // change for every build that predates it.
        let known = local_entry("known", CatalogBackendKind::Acp);
        let future = local_entry("future", CatalogBackendKind::SdkSidecar);
        let catalog = catalog_of(vec![known.clone(), future.clone()]);

        let mut registry = IntegrationRegistry::empty();
        registry
            .register(integration("known", BackendKind::Acp))
            .unwrap();

        let mut resolver = BackendResolver::empty();
        let registration = registry.offer_catalog(&catalog, &mut resolver);
        assert_eq!(registration.registered, std::slice::from_ref(&known.agent));
        assert_eq!(
            registration.skipped,
            [SkippedEntry {
                agent: future.agent.clone(),
                reason: "no_integration",
            }],
            "the skip is reported, not silent"
        );
        assert!(registry.can_serve(&known));
        assert!(!registry.can_serve(&future));

        // Launching it directly is refused for the same reason, with its own
        // code rather than a transport failure further down.
        let error = registry
            .launch(&future, Box::new(ScriptedTransport::default()), &request())
            .unwrap_err();
        assert_eq!(error.code(), "no_integration");

        // A platform this entry does not support is skipped for its own
        // reason, so a UI can say which of the two happened.
        let mut elsewhere = local_entry("elsewhere", CatalogBackendKind::Acp);
        elsewhere.platforms = vec![
            match crate::acp_registry::PlatformTarget::current().unwrap() {
                crate::acp_registry::PlatformTarget::WindowsX86_64 => {
                    crate::acp_registry::PlatformTarget::LinuxX86_64
                }
                _ => crate::acp_registry::PlatformTarget::WindowsX86_64,
            },
        ];
        elsewhere.backend = BackendId::parse("known.backend").unwrap();
        let catalog = catalog_of(vec![elsewhere.clone()]);
        let registration = registry.offer_catalog(&catalog, &mut BackendResolver::empty());
        assert_eq!(
            registration.skipped,
            [SkippedEntry {
                agent: elsewhere.agent,
                reason: "unsupported_platform",
            }]
        );
    }

    #[test]
    fn a_catalog_cannot_break_resolution_of_the_agents_bridge_already_had() {
        // A snapshot arrives from the network. An entry colliding with a
        // backend already registered is skipped — because the alternative is a
        // remote document that can leave Bridge unable to resolve its
        // built-ins, which is a much worse failure than one missing agent.
        let entry = local_entry("known", CatalogBackendKind::Acp);
        let catalog = catalog_of(vec![entry.clone()]);
        let mut registry = IntegrationRegistry::empty();
        registry
            .register(integration("known", BackendKind::Acp))
            .unwrap();

        let mut resolver = BackendResolver::empty();
        resolver
            .register(
                &entry.agent,
                BackendCandidate {
                    backend: entry.backend.clone(),
                    adapter_id: "already-here".into(),
                    kind: BackendKind::Acp,
                },
            )
            .unwrap();

        let registration = registry.offer_catalog(&catalog, &mut resolver);
        assert_eq!(
            registration.skipped,
            [SkippedEntry {
                agent: entry.agent.clone(),
                reason: "backend_conflict",
            }]
        );
        assert!(registration.registered.is_empty());
        assert_eq!(
            resolver
                .candidate(&entry.agent, &entry.backend)
                .unwrap()
                .adapter_id,
            "already-here",
            "the candidate that was already there still serves"
        );
    }

    #[test]
    fn two_integrations_cannot_claim_one_backend() {
        let mut registry = IntegrationRegistry::empty();
        registry
            .register(integration("fake", BackendKind::Acp))
            .unwrap();
        let error = registry
            .register(integration("fake", BackendKind::Acp))
            .unwrap_err();
        assert_eq!(error.code(), "duplicate_integration");
    }

    #[test]
    fn no_integration_method_can_decide_a_permission() {
        // Structural, over the trait's own definition: an integration
        // describes and translates. If a method ever returns a decision, an
        // approval, or a grant, this stops being true.
        let source = include_str!("agent_integration.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        let trait_body = source
            .split_once("pub trait AgentIntegration")
            .expect("the trait must exist")
            .1
            .split_once("\n}")
            .expect("the trait must end")
            .0;
        for forbidden in ["approve", "allow", "grant", "decide", "authorize"] {
            assert!(
                !trait_body.contains(&format!("fn {forbidden}")),
                "AgentIntegration::{forbidden} would let an integration answer its own \
                 permission requests; the caller decides"
            );
        }
        // The one thing it does have is a way to report the model it maps to
        // and the frames it produces — descriptions, not decisions.
        assert!(trait_body.contains("fn normalize"));
        assert!(trait_body.contains("fn permissions"));
    }

    #[test]
    fn a_driver_never_learns_which_agent_it_carries() {
        // The split that makes a second agent on an existing shape a new
        // integration rather than a new driver. A driver signature that
        // mentioned an agent would quietly undo it.
        let source = include_str!("agent_integration.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        let trait_body = source
            .split_once("pub trait BackendDriver")
            .expect("the trait must exist")
            .1
            .split_once("\n}")
            .expect("the trait must end")
            .0;
        for forbidden in ["AgentId", "agent", "VerifiedEntry", "integration"] {
            assert!(
                !trait_body.contains(forbidden),
                "BackendDriver mentions {forbidden:?}: a driver owns a backend shape, \
                 never an agent"
            );
        }
    }
}

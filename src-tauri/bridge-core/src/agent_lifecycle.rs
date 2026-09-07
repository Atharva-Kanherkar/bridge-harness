//! Installation readiness and process lifecycle for managed agents.
//!
//! Three facts about an agent are independent and this module never collapses
//! them into one boolean:
//!
//!   * Bridge owns a receipt-bound payload (`installed`).
//!   * The integration's own non-destructive readiness check passes (`ready`).
//!   * A process is alive right now (`running`).
//!
//! Like [`crate::worker_lifecycle`], the state machine owns transition
//! validation only — callers persist an accepted transition before publishing
//! events. The coordinator above it owns the ordering that keeps the filesystem
//! and the process table honest with each other.
//!
//! # Authentication boundary
//!
//! Vendor authentication stays vendor-owned. A vendor reporting a missing login
//! or API key is not a Bridge failure and is never represented as Bridge
//! credential state: readiness returns [`ReadinessOutcome::VendorBlocked`], the
//! vendor's own message is preserved verbatim, and the agent stays `installed`
//! rather than being recast as `broken` or promoted to `ready`. This module has
//! no credential store, no OAuth client, and no logout path.

use crate::adapters::ShutdownReason;
use crate::managed_payload::{
    inspect_external_runtime, ManagedPayloadStatus, ManagedPayloadStore, PayloadRecipe,
    RepairReason,
};
use crate::secret_interception;
use crate::BridgeError;
use std::{
    collections::HashMap,
    error::Error,
    fmt,
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentLifecycleState {
    /// No Bridge-managed payload and no discoverable user-managed runtime.
    NotInstalled,
    /// A managed install is in flight and may still be cancelled.
    Installing,
    /// Bridge owns a receipt-bound payload; launch readiness is not yet proven.
    Installed,
    /// The integration's own readiness check passed. Spawn-on-use starts here.
    Ready,
    /// A live process exists.
    Running,
    /// A stop was requested and the process has not been reaped yet.
    Stopping,
    /// A managed payload is being removed.
    Uninstalling,
    /// A user-managed runtime is discoverable. Bridge holds no receipt for it
    /// and must never remove it.
    External,
    /// Unusable for a reason repair cannot be expected to fix on its own.
    Broken,
    /// Bridge owns the payload but it drifted from its receipt.
    Repairable,
}

impl AgentLifecycleState {
    pub const ALL: [Self; 10] = [
        Self::NotInstalled,
        Self::Installing,
        Self::Installed,
        Self::Ready,
        Self::Running,
        Self::Stopping,
        Self::Uninstalling,
        Self::External,
        Self::Broken,
        Self::Repairable,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotInstalled => "not_installed",
            Self::Installing => "installing",
            Self::Installed => "installed",
            Self::Ready => "ready",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Uninstalling => "uninstalling",
            Self::External => "external",
            Self::Broken => "broken",
            Self::Repairable => "repairable",
        }
    }

    /// Does Bridge hold, or expect to hold, a receipt for this agent's payload?
    ///
    /// False for `external`, where a discoverable user-managed runtime is usable
    /// but not Bridge's to remove, and false for `installing`, where staging has
    /// not been promoted yet so there is nothing to own. `broken` is owned: it is
    /// only reachable from states that already had a managed payload, which is
    /// why removing a broken install is allowed.
    pub const fn is_bridge_owned(self) -> bool {
        matches!(
            self,
            Self::Installed
                | Self::Ready
                | Self::Running
                | Self::Stopping
                | Self::Uninstalling
                | Self::Repairable
                | Self::Broken
        )
    }

    /// Might a process still be alive in this state?
    pub const fn may_have_process(self) -> bool {
        matches!(self, Self::Running | Self::Stopping)
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        LEGAL_TRANSITIONS.contains(&(self, next))
    }
}

impl fmt::Display for AgentLifecycleState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for AgentLifecycleState {
    type Err = ParseAgentLifecycleStateError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|state| state.as_str() == value)
            .ok_or_else(|| ParseAgentLifecycleStateError(value.to_owned()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseAgentLifecycleStateError(pub String);

impl fmt::Display for ParseAgentLifecycleStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown agent lifecycle state: {}", self.0)
    }
}

impl Error for ParseAgentLifecycleStateError {}

/// The complete transition relation.
///
/// Two absences are load-bearing rather than oversights, and both are asserted
/// in the tests below:
///
///   * `running` has no edge to `uninstalling`. Removing a payload underneath a
///     live process, or handing a fresh launch a path that is being deleted, is
///     only prevented if stopping is a state the machine must pass through.
///   * `external` has no edge to `uninstalling`. Bridge holds no receipt for a
///     user-managed runtime, so there is no state from which it could begin
///     removing one.
pub const LEGAL_TRANSITIONS: [(AgentLifecycleState, AgentLifecycleState); 32] = [
    // Discovery.
    (
        AgentLifecycleState::NotInstalled,
        AgentLifecycleState::Installing,
    ),
    (
        AgentLifecycleState::NotInstalled,
        AgentLifecycleState::External,
    ),
    (
        AgentLifecycleState::External,
        AgentLifecycleState::NotInstalled,
    ),
    // Opting into a managed payload alongside a user-managed runtime.
    (
        AgentLifecycleState::External,
        AgentLifecycleState::Installing,
    ),
    // Install, including cancellation back to where it started.
    (
        AgentLifecycleState::Installing,
        AgentLifecycleState::Installed,
    ),
    (
        AgentLifecycleState::Installing,
        AgentLifecycleState::NotInstalled,
    ),
    (
        AgentLifecycleState::Installing,
        AgentLifecycleState::Repairable,
    ),
    (AgentLifecycleState::Installing, AgentLifecycleState::Broken),
    // Readiness is a separate promotion from ownership.
    (AgentLifecycleState::Installed, AgentLifecycleState::Ready),
    (
        AgentLifecycleState::Installed,
        AgentLifecycleState::Repairable,
    ),
    (AgentLifecycleState::Installed, AgentLifecycleState::Broken),
    (
        AgentLifecycleState::Installed,
        AgentLifecycleState::Uninstalling,
    ),
    // Spawn-on-use.
    (AgentLifecycleState::Ready, AgentLifecycleState::Running),
    (AgentLifecycleState::Ready, AgentLifecycleState::Installed),
    (AgentLifecycleState::Ready, AgentLifecycleState::Repairable),
    (AgentLifecycleState::Ready, AgentLifecycleState::Broken),
    (
        AgentLifecycleState::Ready,
        AgentLifecycleState::Uninstalling,
    ),
    // Run and stop.
    (AgentLifecycleState::Running, AgentLifecycleState::Stopping),
    (AgentLifecycleState::Running, AgentLifecycleState::Ready),
    (AgentLifecycleState::Running, AgentLifecycleState::Broken),
    (AgentLifecycleState::Stopping, AgentLifecycleState::Ready),
    (
        AgentLifecycleState::Stopping,
        AgentLifecycleState::Uninstalling,
    ),
    (AgentLifecycleState::Stopping, AgentLifecycleState::Broken),
    // Uninstall, including a refusal that leaves the payload intact.
    (
        AgentLifecycleState::Uninstalling,
        AgentLifecycleState::NotInstalled,
    ),
    (
        AgentLifecycleState::Uninstalling,
        AgentLifecycleState::Installed,
    ),
    (
        AgentLifecycleState::Uninstalling,
        AgentLifecycleState::Broken,
    ),
    // Repair.
    (
        AgentLifecycleState::Repairable,
        AgentLifecycleState::Installing,
    ),
    (
        AgentLifecycleState::Repairable,
        AgentLifecycleState::Installed,
    ),
    (
        AgentLifecycleState::Repairable,
        AgentLifecycleState::Uninstalling,
    ),
    // Retry and removal out of a broken install.
    (AgentLifecycleState::Broken, AgentLifecycleState::Installing),
    (
        AgentLifecycleState::Broken,
        AgentLifecycleState::Uninstalling,
    ),
    (
        AgentLifecycleState::Broken,
        AgentLifecycleState::NotInstalled,
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidAgentLifecycleTransition {
    pub from: AgentLifecycleState,
    pub to: AgentLifecycleState,
}

impl fmt::Display for InvalidAgentLifecycleTransition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "illegal agent lifecycle transition: {} -> {}",
            self.from, self.to
        )
    }
}

impl Error for InvalidAgentLifecycleTransition {}

pub fn validate_transition(
    from: AgentLifecycleState,
    to: AgentLifecycleState,
) -> Result<(), InvalidAgentLifecycleTransition> {
    if from.can_transition_to(to) {
        Ok(())
    } else {
        Err(InvalidAgentLifecycleTransition { from, to })
    }
}

/// What the #165 payload engine says about the payload, reduced to what the
/// lifecycle needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayloadCondition {
    /// No Bridge-managed payload.
    Absent,
    /// Bridge owns a receipt-bound payload with a verified entrypoint.
    Installed { entrypoint: PathBuf },
    /// Bridge owns the payload but it no longer matches its receipt.
    Repairable { reason: RepairReason },
}

impl PayloadCondition {
    /// Reduce a [`ManagedPayloadStatus`] without reinterpreting it.
    ///
    /// Every repair reason stays a repair reason, including
    /// `EntrypointNotExecutable`: an installed payload that cannot be executed
    /// is drift to be repaired, never a `ready` agent.
    pub fn from_status(status: ManagedPayloadStatus) -> Self {
        match status {
            ManagedPayloadStatus::NotInstalled => Self::Absent,
            ManagedPayloadStatus::Installed { entrypoint, .. } => Self::Installed { entrypoint },
            ManagedPayloadStatus::Repairable { reason } => Self::Repairable { reason },
        }
    }
}

/// A user-managed runtime Bridge can see but does not own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalRuntime {
    pub candidate: PathBuf,
}

/// The result of an integration's own non-destructive readiness check.
///
/// Bridge does not implement these checks — Claude's is a Node and sidecar
/// probe, Codex's is a `codex --version`, OpenCode's is its provider catalog —
/// and this enum exists so their verdicts can be carried without being
/// rewritten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadinessOutcome {
    /// The integration can launch.
    Ready { version: Option<String> },
    /// The integration cannot launch for a reason reinstalling will not fix — a
    /// missing runtime prerequisite, say.
    Unavailable { reason: String },
    /// The vendor reported that it needs a login or an API key.
    ///
    /// Deliberately distinct from [`Self::Unavailable`]: nothing is broken, the
    /// user simply has not authenticated with the vendor. `vendor_message` is
    /// the vendor's own text, carried verbatim, and Bridge stores no credential
    /// state of its own for it.
    VendorBlocked { vendor_message: String },
    /// Not probed — there was no payload to probe.
    NotProbed,
}

impl ReadinessOutcome {
    /// Is this verdict the vendor's rather than Bridge's?
    ///
    /// Callers use this to keep a vendor auth prompt recognizable as a vendor
    /// concern instead of reporting it as a Bridge failure.
    pub const fn is_vendor_owned(&self) -> bool {
        matches!(self, Self::VendorBlocked { .. })
    }

    /// The message to show, if any. Vendor text is never reworded.
    pub fn message(&self) -> Option<&str> {
        match self {
            Self::Ready { .. } | Self::NotProbed => None,
            Self::Unavailable { reason } => Some(reason),
            Self::VendorBlocked { vendor_message } => Some(vendor_message),
        }
    }
}

/// Whether a process is alive right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessCondition {
    None,
    Running { pid: u32 },
}

/// The three independent facts, gathered from sources this module does not own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleObservation {
    pub payload: PayloadCondition,
    pub external: Option<ExternalRuntime>,
    pub readiness: ReadinessOutcome,
    pub process: ProcessCondition,
}

impl LifecycleObservation {
    pub fn absent() -> Self {
        Self {
            payload: PayloadCondition::Absent,
            external: None,
            readiness: ReadinessOutcome::NotProbed,
            process: ProcessCondition::None,
        }
    }

    /// Resolve the settled state these facts describe.
    ///
    /// "Settled" means no operation is in flight: `installing`, `stopping`, and
    /// `uninstalling` are owned by the coordinator driving an operation and are
    /// never inferred from an observation.
    ///
    /// Precedence, in order, and each rule is load-bearing:
    ///
    /// 1. **A live process resolves to `running`, whatever the payload says.**
    ///    `stopping` is only reachable from `running`, so any other answer would
    ///    leave a live process in a state Bridge cannot legally stop it from. A
    ///    payload that drifted or vanished underneath a running process becomes
    ///    actionable once the process is reaped, and until then the observation
    ///    still carries the payload condition for callers that want to warn.
    /// 2. Drift outranks readiness. A `repairable` payload is never `ready`.
    /// 3. A Bridge-managed payload outranks a discoverable external runtime,
    ///    because the managed payload is the one Bridge would launch. `external`
    ///    describes the case where there is nothing managed to launch.
    /// 4. Readiness promotes `installed` to `ready`. A vendor auth block leaves
    ///    the agent `installed` — owned, not launch-ready, nothing broken.
    pub fn settled_state(&self) -> AgentLifecycleState {
        if matches!(self.process, ProcessCondition::Running { .. }) {
            return AgentLifecycleState::Running;
        }
        match &self.payload {
            PayloadCondition::Repairable { .. } => AgentLifecycleState::Repairable,
            PayloadCondition::Absent => {
                if self.external.is_some() {
                    AgentLifecycleState::External
                } else {
                    AgentLifecycleState::NotInstalled
                }
            }
            PayloadCondition::Installed { .. } => match &self.readiness {
                ReadinessOutcome::Ready { .. } => AgentLifecycleState::Ready,
                ReadinessOutcome::Unavailable { .. } => AgentLifecycleState::Broken,
                ReadinessOutcome::VendorBlocked { .. } | ReadinessOutcome::NotProbed => {
                    AgentLifecycleState::Installed
                }
            },
        }
    }
}

/// How many consecutive failures a agent may accumulate before Bridge stops
/// relaunching it on its own.
pub const DEFAULT_MAX_CONSECUTIVE_FAILURES: u32 = 3;
/// Ceiling on a retained failure context. A stderr tail is already bounded by
/// [`crate::adapters::StderrTail`]; this bounds anything else a caller hands in.
pub const FAILURE_CONTEXT_MAX_BYTES: usize = 2_000;

/// Why an agent failed, in a form that is safe to keep in lifecycle state.
///
/// The context has been through [`secret_interception::sanitize`] and truncated,
/// because the most useful failure context available — a provider's stderr tail —
/// is also the most likely place for a token to appear.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactedFailure {
    pub context: String,
    /// Which consecutive attempt this was, 1-based.
    pub attempt: u32,
}

/// A bounded budget for consecutive failures, so a crash loop stops on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureBudget {
    max_consecutive: u32,
    consecutive: u32,
    last: Option<RedactedFailure>,
}

impl Default for FailureBudget {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_CONSECUTIVE_FAILURES)
    }
}

impl FailureBudget {
    pub const fn new(max_consecutive: u32) -> Self {
        Self {
            max_consecutive,
            consecutive: 0,
            last: None,
        }
    }

    pub const fn consecutive(&self) -> u32 {
        self.consecutive
    }

    pub const fn max_consecutive(&self) -> u32 {
        self.max_consecutive
    }

    /// Has Bridge stopped relaunching this agent on its own?
    pub const fn is_exhausted(&self) -> bool {
        self.consecutive >= self.max_consecutive
    }

    /// May Bridge attempt another launch without an explicit operator retry?
    pub const fn may_retry(&self) -> bool {
        !self.is_exhausted()
    }

    pub const fn last_failure(&self) -> Option<&RedactedFailure> {
        self.last.as_ref()
    }

    /// Record a failure, redacting and bounding its context.
    ///
    /// Counting saturates so a long-lived agent cannot wrap the counter back
    /// into a state where Bridge would start relaunching it again.
    pub fn record_failure(&mut self, context: Option<&str>) -> &RedactedFailure {
        self.consecutive = self.consecutive.saturating_add(1);
        self.last = Some(RedactedFailure {
            context: redact_failure_context(context.unwrap_or("no failure context reported")),
            attempt: self.consecutive,
        });
        self.last.as_ref().expect("failure was just recorded")
    }

    /// A run got far enough to count as working. Clears the streak but keeps the
    /// last failure, which is still the useful thing to show after a flap.
    pub fn record_success(&mut self) {
        self.consecutive = 0;
    }

    /// An operator asked to try again, which is the only thing that clears an
    /// exhausted budget.
    pub fn reset(&mut self) {
        self.consecutive = 0;
        self.last = None;
    }
}

/// Redact and bound a failure context.
fn redact_failure_context(raw: &str) -> String {
    let mut redacted = secret_interception::sanitize(raw).text;
    if redacted.len() > FAILURE_CONTEXT_MAX_BYTES {
        let mut cut = FAILURE_CONTEXT_MAX_BYTES;
        while cut > 0 && !redacted.is_char_boundary(cut) {
            cut -= 1;
        }
        redacted.truncate(cut);
        redacted.push('…');
    }
    redacted
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentLifecycle {
    state: AgentLifecycleState,
}

impl AgentLifecycle {
    pub const fn new(state: AgentLifecycleState) -> Self {
        Self { state }
    }

    pub const fn state(self) -> AgentLifecycleState {
        self.state
    }

    pub fn transition_to(
        &mut self,
        next: AgentLifecycleState,
    ) -> Result<(), InvalidAgentLifecycleTransition> {
        validate_transition(self.state, next)?;
        self.state = next;
        Ok(())
    }
}

/// Whether an in-flight install has been cancelled.
///
/// A trait rather than a bare `&AtomicBool` so the two poll sites — before
/// staging and after promotion — can be told apart in tests. Real callers pass
/// an `AtomicBool`.
pub trait CancellationSignal: Send + Sync {
    fn is_cancelled(&self) -> bool;
}

impl CancellationSignal for AtomicBool {
    fn is_cancelled(&self) -> bool {
        self.load(Ordering::SeqCst)
    }
}

/// A process the coordinator launched and is now responsible for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchedProcess {
    pub pid: u32,
}

/// An integration's own non-destructive readiness check.
///
/// Bridge does not reimplement these. Claude probes Node and its sidecar entry,
/// Codex runs `codex --version`, OpenCode reads its provider catalog; this trait
/// only carries their existing verdicts into the lifecycle.
pub trait ReadinessProbe: Send + Sync {
    fn probe(&self, agent_id: &str, entrypoint: &Path) -> ReadinessOutcome;
}

/// Spawn-on-use, liveness, and clean stop for one agent's process.
pub trait ProcessSupervisor: Send + Sync {
    fn launch(&self, agent_id: &str, entrypoint: &Path) -> Result<LaunchedProcess, String>;
    fn is_running(&self, pid: u32) -> bool;
    /// Request that a process stop. The coordinator confirms the result through
    /// [`Self::is_running`] before it clears process state or removes a payload.
    fn stop(&self, pid: u32, reason: ShutdownReason) -> bool;
    /// Escalate an app-shutdown stop and wait for the process to reap.
    ///
    /// Called only after [`Self::stop`] left a process live. Returning `true`
    /// is not itself proof of success; the coordinator checks liveness again.
    fn force_stop(&self, pid: u32, reason: ShutdownReason) -> bool;
}

/// What a caller sees. Nothing here represents vendor credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStatus {
    pub agent_id: String,
    pub state: AgentLifecycleState,
    pub readiness: ReadinessOutcome,
    pub external: Option<ExternalRuntime>,
    pub process_id: Option<u32>,
    pub consecutive_failures: u32,
    pub last_failure: Option<RedactedFailure>,
}

impl AgentStatus {
    /// The vendor's own message, when the vendor is what is blocking.
    ///
    /// Separate from [`Self::last_failure`] so a caller cannot mistake "needs a
    /// vendor login" for "Bridge failed".
    pub fn vendor_message(&self) -> Option<&str> {
        self.readiness
            .is_vendor_owned()
            .then(|| self.readiness.message())
            .flatten()
    }
}

#[derive(Debug)]
pub enum LifecycleError {
    Payload(BridgeError),
    Transition(InvalidAgentLifecycleTransition),
    /// A user-managed runtime is not Bridge's to remove. Deliberately distinct
    /// from a generic invalid transition so a caller can say why.
    ExternalRuntimeNotRemovable {
        agent_id: String,
        candidate: PathBuf,
    },
    NotReadyToLaunch {
        agent_id: String,
        state: AgentLifecycleState,
    },
    RetryBudgetExhausted {
        agent_id: String,
        attempts: u32,
    },
    LaunchFailed {
        agent_id: String,
        context: String,
    },
    StopFailed {
        agent_id: String,
        pid: u32,
    },
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Payload(error) => write!(formatter, "{error}"),
            Self::Transition(error) => write!(formatter, "{error}"),
            Self::ExternalRuntimeNotRemovable {
                agent_id,
                candidate,
            } => write!(
                formatter,
                "{agent_id} is a user-managed runtime at {} — Bridge holds no receipt for it and will not remove it",
                candidate.display()
            ),
            Self::NotReadyToLaunch { agent_id, state } => write!(
                formatter,
                "{agent_id} cannot be launched from {state}; only a ready agent can be launched"
            ),
            Self::RetryBudgetExhausted { agent_id, attempts } => write!(
                formatter,
                "{agent_id} failed {attempts} times in a row; retry must be requested explicitly"
            ),
            Self::LaunchFailed { agent_id, context } => {
                write!(formatter, "{agent_id} failed to launch: {context}")
            }
            Self::StopFailed { agent_id, pid } => {
                write!(formatter, "{agent_id} process {pid} did not stop")
            }
        }
    }
}

impl Error for LifecycleError {}

impl From<InvalidAgentLifecycleTransition> for LifecycleError {
    fn from(error: InvalidAgentLifecycleTransition) -> Self {
        Self::Transition(error)
    }
}

impl From<BridgeError> for LifecycleError {
    fn from(error: BridgeError) -> Self {
        Self::Payload(error)
    }
}

/// Per-agent bookkeeping the coordinator owns.
#[derive(Debug)]
struct AgentRecord {
    /// Set only while the coordinator is driving an operation. When `None`, the
    /// agent's state is whatever a fresh observation says it is.
    in_flight: Option<AgentLifecycleState>,
    budget: FailureBudget,
    process: Option<LaunchedProcess>,
    last_activity: Instant,
}

impl Default for AgentRecord {
    fn default() -> Self {
        Self {
            in_flight: None,
            budget: FailureBudget::default(),
            process: None,
            last_activity: Instant::now(),
        }
    }
}

/// Drives installation, readiness, and process lifecycle for managed agents.
///
/// # Why observed state is derived rather than converged
///
/// Only `installing`, `stopping`, and `uninstalling` are stored, because only
/// those describe Bridge actively doing something. Every other state is computed
/// from a fresh observation.
///
/// The alternative — storing the last state and walking legal edges toward each
/// new observation — forces the machine to lie. A payload that vanishes out of
/// band would have to be reported as having passed through `uninstalling`, which
/// says Bridge removed it. Deriving instead means an observation is never
/// dressed up as an operation, and the transition matrix still governs every
/// edge the coordinator itself drives.
pub struct AgentLifecycleCoordinator {
    store: ManagedPayloadStore,
    probe: Arc<dyn ReadinessProbe>,
    supervisor: Arc<dyn ProcessSupervisor>,
    /// Opt-in. `None` means a long-untouched process is left alone.
    idle_timeout: Option<Duration>,
    records: Mutex<HashMap<String, AgentRecord>>,
    /// One lock per agent, so a launch cannot interleave with a removal.
    ///
    /// Without this, `ensure_running` could observe `ready`, an `uninstall`
    /// could complete, and the launch would then be handed an entrypoint that no
    /// longer exists — the second half of the issue's race requirement, which
    /// stopping first does not address on its own.
    operations: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl AgentLifecycleCoordinator {
    pub fn new(
        store: ManagedPayloadStore,
        probe: Arc<dyn ReadinessProbe>,
        supervisor: Arc<dyn ProcessSupervisor>,
        idle_timeout: Option<Duration>,
    ) -> Self {
        Self {
            store,
            probe,
            supervisor,
            idle_timeout,
            records: Mutex::new(HashMap::new()),
            operations: Mutex::new(HashMap::new()),
        }
    }

    /// Serialize operations for one agent.
    ///
    /// Poisoning is tolerated: the lock guards ordering, not data, so a panic
    /// under it leaves nothing inconsistent and refusing every later operation
    /// would be strictly worse.
    fn operation_lock(&self, agent_id: &str) -> Arc<Mutex<()>> {
        self.operations
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entry(agent_id.to_owned())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    #[cfg(test)]
    fn store(&self) -> &ManagedPayloadStore {
        &self.store
    }

    fn records(&self) -> std::sync::MutexGuard<'_, HashMap<String, AgentRecord>> {
        // The map guards bookkeeping with no invariant a panic could corrupt,
        // so poisoning is tolerated rather than bricking every later operation.
        self.records
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    /// Gather the three facts and the state they describe.
    pub fn observe(
        &self,
        agent_id: &str,
        external_candidates: &[PathBuf],
    ) -> Result<LifecycleObservation, LifecycleError> {
        let payload = PayloadCondition::from_status(self.store.status(agent_id)?);
        let external = external_candidates
            .iter()
            .map(inspect_external_runtime)
            .find(|inspection| inspection.available)
            .map(|inspection| ExternalRuntime {
                candidate: inspection.candidate,
            });
        let readiness = match &payload {
            PayloadCondition::Installed { entrypoint } => self.probe.probe(agent_id, entrypoint),
            PayloadCondition::Absent | PayloadCondition::Repairable { .. } => {
                ReadinessOutcome::NotProbed
            }
        };
        let process = self.live_process(agent_id);
        Ok(LifecycleObservation {
            payload,
            external,
            readiness,
            process,
        })
    }

    /// A tracked process is only reported as running if it is actually alive, so
    /// a reaped child cannot keep an agent pinned in `running`.
    fn live_process(&self, agent_id: &str) -> ProcessCondition {
        let mut records = self.records();
        let Some(record) = records.get_mut(agent_id) else {
            return ProcessCondition::None;
        };
        match record.process {
            Some(process) if self.supervisor.is_running(process.pid) => {
                ProcessCondition::Running { pid: process.pid }
            }
            Some(_) => {
                record.process = None;
                ProcessCondition::None
            }
            None => ProcessCondition::None,
        }
    }

    pub fn status(
        &self,
        agent_id: &str,
        external_candidates: &[PathBuf],
    ) -> Result<AgentStatus, LifecycleError> {
        let observation = self.observe(agent_id, external_candidates)?;
        Ok(self.status_from(agent_id, observation))
    }

    fn status_from(&self, agent_id: &str, observation: LifecycleObservation) -> AgentStatus {
        let records = self.records();
        let record = records.get(agent_id);
        let settled = observation.settled_state();
        let state = match record.and_then(|record| record.in_flight) {
            Some(in_flight) => in_flight,
            // An agent Bridge has stopped relaunching must not claim to be
            // `ready`: the payload is fine, but nothing will start it until an
            // operator retries, and reporting readiness would invite a caller to
            // keep asking. Both edges used here are in the matrix.
            None if record.is_some_and(|record| record.budget.is_exhausted())
                && settled == AgentLifecycleState::Ready =>
            {
                AgentLifecycleState::Broken
            }
            None => settled,
        };
        AgentStatus {
            agent_id: agent_id.to_owned(),
            state,
            readiness: observation.readiness,
            external: observation.external,
            process_id: match observation.process {
                ProcessCondition::Running { pid } => Some(pid),
                ProcessCondition::None => None,
            },
            consecutive_failures: record
                .map(|record| record.budget.consecutive())
                .unwrap_or(0),
            last_failure: record.and_then(|record| record.budget.last_failure().cloned()),
        }
    }

    /// The state right now, for guarding an operation.
    fn current_state(
        &self,
        agent_id: &str,
        external_candidates: &[PathBuf],
    ) -> Result<AgentLifecycleState, LifecycleError> {
        Ok(self.status(agent_id, external_candidates)?.state)
    }

    fn begin(
        &self,
        agent_id: &str,
        from: AgentLifecycleState,
        to: AgentLifecycleState,
    ) -> Result<(), LifecycleError> {
        validate_transition(from, to)?;
        let mut records = self.records();
        records.entry(agent_id.to_owned()).or_default().in_flight = Some(to);
        Ok(())
    }

    /// Leave an in-flight state through a legal edge.
    ///
    /// The edge is validated even though the in-flight state is about to be
    /// dropped, so an operation cannot finish somewhere the matrix forbids.
    fn finish(
        &self,
        agent_id: &str,
        from: AgentLifecycleState,
        to: AgentLifecycleState,
    ) -> Result<(), LifecycleError> {
        validate_transition(from, to)?;
        let mut records = self.records();
        if let Some(record) = records.get_mut(agent_id) {
            record.in_flight = None;
        }
        Ok(())
    }

    /// Clear an `installing` operation after a store or rollback failure.
    ///
    /// The payload engine remains the source of truth for whether anything
    /// landed. Every target here is an explicit edge out of `installing`, so an
    /// error can never strand an agent in a synthetic in-flight state.
    fn finish_failed_install(&self, agent_id: &str) -> Result<(), LifecycleError> {
        let settled = match self.store.status(agent_id) {
            Ok(ManagedPayloadStatus::NotInstalled) => AgentLifecycleState::NotInstalled,
            Ok(ManagedPayloadStatus::Installed { .. }) => AgentLifecycleState::Installed,
            Ok(ManagedPayloadStatus::Repairable { .. }) => AgentLifecycleState::Repairable,
            Err(_) => AgentLifecycleState::Broken,
        };
        self.finish(agent_id, AgentLifecycleState::Installing, settled)
    }

    /// Install a managed payload.
    ///
    /// `cancelled` is polled before staging and again after promotion; a
    /// cancellation observed after the payload landed is rolled back through the
    /// engine's own uninstall, so cancelling never leaves an active receipt.
    pub fn install(
        &self,
        recipe: &PayloadRecipe,
        external_candidates: &[PathBuf],
        cancelled: &dyn CancellationSignal,
    ) -> Result<AgentStatus, LifecycleError> {
        let lock = self.operation_lock(&recipe.agent_id);
        let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
        self.install_locked(recipe, external_candidates, cancelled)
    }

    fn install_locked(
        &self,
        recipe: &PayloadRecipe,
        external_candidates: &[PathBuf],
        cancelled: &dyn CancellationSignal,
    ) -> Result<AgentStatus, LifecycleError> {
        let agent_id = recipe.agent_id.as_str();
        let from = self.current_state(agent_id, external_candidates)?;
        self.begin(agent_id, from, AgentLifecycleState::Installing)?;

        if cancelled.is_cancelled() {
            self.finish(
                agent_id,
                AgentLifecycleState::Installing,
                AgentLifecycleState::NotInstalled,
            )?;
            return self.status(agent_id, external_candidates);
        }

        // A receipt-owned payload that has drifted must use the engine's repair
        // operation. `install` deliberately refuses to overwrite that payload,
        // because it needs its existing receipt as ownership proof.
        let installed = if from == AgentLifecycleState::Repairable {
            self.store.repair(recipe).map(|_| ())
        } else {
            self.store.install(recipe).map(|_| ())
        };
        let outcome = match installed {
            Ok(_) if cancelled.is_cancelled() => {
                // Landed, then cancelled: roll back so no active receipt survives.
                if let Err(error) = self.store.uninstall(agent_id) {
                    self.finish_failed_install(agent_id)?;
                    return Err(error.into());
                }
                self.finish(
                    agent_id,
                    AgentLifecycleState::Installing,
                    AgentLifecycleState::NotInstalled,
                )?;
                return self.status(agent_id, external_candidates);
            }
            Ok(_) => {
                let observation = match self.observe(agent_id, external_candidates) {
                    Ok(observation) => observation,
                    Err(error) => {
                        self.finish_failed_install(agent_id)?;
                        return Err(error);
                    }
                };
                match observation.settled_state() {
                    AgentLifecycleState::Repairable => AgentLifecycleState::Repairable,
                    _ => AgentLifecycleState::Installed,
                }
            }
            Err(error) => {
                self.finish_failed_install(agent_id)?;
                return Err(error.into());
            }
        };
        self.finish(agent_id, AgentLifecycleState::Installing, outcome)?;
        self.status(agent_id, external_candidates)
    }

    /// Spawn-on-use. Only a `ready` agent launches.
    pub fn ensure_running(
        &self,
        agent_id: &str,
        external_candidates: &[PathBuf],
    ) -> Result<AgentStatus, LifecycleError> {
        let lock = self.operation_lock(agent_id);
        let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
        self.ensure_running_locked(agent_id, external_candidates)
    }

    fn ensure_running_locked(
        &self,
        agent_id: &str,
        external_candidates: &[PathBuf],
    ) -> Result<AgentStatus, LifecycleError> {
        let observation = self.observe(agent_id, external_candidates)?;
        let state = self.status_from(agent_id, observation.clone()).state;
        if state == AgentLifecycleState::Running {
            self.touch(agent_id);
            return Ok(self.status_from(agent_id, observation));
        }
        // A spent budget is reported before the state, because it is the more
        // specific and more actionable reason: the payload is fine and an
        // explicit retry is all that is needed. Checking the state first would
        // report the `broken` that the spent budget itself produced.
        {
            let records = self.records();
            if let Some(record) = records.get(agent_id) {
                if !record.budget.may_retry() {
                    return Err(LifecycleError::RetryBudgetExhausted {
                        agent_id: agent_id.to_owned(),
                        attempts: record.budget.consecutive(),
                    });
                }
            }
        }
        if state != AgentLifecycleState::Ready {
            return Err(LifecycleError::NotReadyToLaunch {
                agent_id: agent_id.to_owned(),
                state,
            });
        }
        let PayloadCondition::Installed { entrypoint } = &observation.payload else {
            return Err(LifecycleError::NotReadyToLaunch {
                agent_id: agent_id.to_owned(),
                state,
            });
        };
        validate_transition(AgentLifecycleState::Ready, AgentLifecycleState::Running)?;
        match self.supervisor.launch(agent_id, entrypoint) {
            Ok(process) => {
                let mut records = self.records();
                let record = records.entry(agent_id.to_owned()).or_default();
                record.process = Some(process);
                record.last_activity = Instant::now();
                record.budget.record_success();
                drop(records);
                self.status(agent_id, external_candidates)
            }
            Err(context) => {
                let mut records = self.records();
                let record = records.entry(agent_id.to_owned()).or_default();
                let redacted = record.budget.record_failure(Some(&context)).context.clone();
                drop(records);
                Err(LifecycleError::LaunchFailed {
                    agent_id: agent_id.to_owned(),
                    context: redacted,
                })
            }
        }
    }

    /// Record that a launched process exited.
    ///
    /// A clean exit returns the agent to whatever a fresh observation says —
    /// `ready`, normally, so spawn-on-use can start it again. A failure counts
    /// against the crash-loop budget and keeps redacted context.
    pub fn note_process_exit(
        &self,
        agent_id: &str,
        pid: u32,
        failure_context: Option<&str>,
    ) -> Result<(), LifecycleError> {
        let mut records = self.records();
        let Some(record) = records.get_mut(agent_id) else {
            return Ok(());
        };
        // Exit reporting can be delayed. A notification for an already-reaped
        // process must not clear a replacement process that has since launched.
        match record.process {
            Some(process) if process.pid == pid => {}
            Some(_) | None => return Ok(()),
        }
        record.process = None;
        match failure_context {
            Some(context) => {
                record.budget.record_failure(Some(context));
            }
            None => record.budget.record_success(),
        }
        Ok(())
    }

    /// An explicit operator retry, the only thing that clears an exhausted budget.
    pub fn reset_retry_budget(&self, agent_id: &str) {
        let mut records = self.records();
        records
            .entry(agent_id.to_owned())
            .or_default()
            .budget
            .reset();
    }

    fn touch(&self, agent_id: &str) {
        let mut records = self.records();
        records
            .entry(agent_id.to_owned())
            .or_default()
            .last_activity = Instant::now();
    }

    /// Stop a running agent through `running → stopping → ready`.
    pub fn stop(
        &self,
        agent_id: &str,
        reason: ShutdownReason,
        external_candidates: &[PathBuf],
    ) -> Result<AgentStatus, LifecycleError> {
        let lock = self.operation_lock(agent_id);
        let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
        self.stop_locked(agent_id, reason, external_candidates)
    }

    fn stop_locked(
        &self,
        agent_id: &str,
        reason: ShutdownReason,
        external_candidates: &[PathBuf],
    ) -> Result<AgentStatus, LifecycleError> {
        let pid = match self.live_process(agent_id) {
            ProcessCondition::Running { pid } => pid,
            ProcessCondition::None => return self.status(agent_id, external_candidates),
        };
        // Derive the state to leave rather than assuming `running`. A live
        // process normally resolves to `running`, but if another operation is
        // in flight the stored state wins, and interleaving a stop into it would
        // step outside the matrix.
        let from = self.status(agent_id, external_candidates)?.state;
        if from != AgentLifecycleState::Running {
            return Err(LifecycleError::Transition(
                InvalidAgentLifecycleTransition {
                    from,
                    to: AgentLifecycleState::Stopping,
                },
            ));
        }
        self.begin(agent_id, from, AgentLifecycleState::Stopping)?;
        let _stop_requested = self.supervisor.stop(pid, reason);
        // A supervisor acknowledgement is not proof that the child is gone.
        // Keep tracking the PID until liveness says otherwise; otherwise an
        // uninstall could delete the payload beneath a still-running process.
        let still_running = self.supervisor.is_running(pid);
        {
            let mut records = self.records();
            if let Some(record) = records.get_mut(agent_id) {
                if !still_running {
                    record.process = None;
                }
            }
        }
        if still_running {
            self.finish(
                agent_id,
                AgentLifecycleState::Stopping,
                AgentLifecycleState::Broken,
            )?;
            return Err(LifecycleError::StopFailed {
                agent_id: agent_id.to_owned(),
                pid,
            });
        }
        self.finish(
            agent_id,
            AgentLifecycleState::Stopping,
            AgentLifecycleState::Ready,
        )?;
        self.status(agent_id, external_candidates)
    }

    /// Remove a Bridge-managed payload.
    ///
    /// Refuses a user-managed runtime outright, and stops a live process before
    /// touching the filesystem so no process is left pointing at deleted files
    /// and no fresh launch can be handed a path being removed.
    pub fn uninstall(
        &self,
        agent_id: &str,
        external_candidates: &[PathBuf],
    ) -> Result<AgentStatus, LifecycleError> {
        let lock = self.operation_lock(agent_id);
        let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
        self.uninstall_locked(agent_id, external_candidates)
    }

    fn uninstall_locked(
        &self,
        agent_id: &str,
        external_candidates: &[PathBuf],
    ) -> Result<AgentStatus, LifecycleError> {
        let observation = self.observe(agent_id, external_candidates)?;
        let state = self.status_from(agent_id, observation.clone()).state;

        if state == AgentLifecycleState::External {
            let candidate = observation
                .external
                .map(|external| external.candidate)
                .unwrap_or_default();
            return Err(LifecycleError::ExternalRuntimeNotRemovable {
                agent_id: agent_id.to_owned(),
                candidate,
            });
        }
        if state == AgentLifecycleState::NotInstalled {
            return self.status(agent_id, external_candidates);
        }

        // Stop first. The matrix has no running -> uninstalling edge, so this is
        // the only way through, and it is what keeps the removal off a live
        // process.
        let state = if state == AgentLifecycleState::Running {
            self.stop_locked(agent_id, ShutdownReason::Replaced, external_candidates)?
                .state
        } else {
            state
        };
        // Re-check liveness rather than trusting the state we just computed: the
        // whole point of stopping first is that the payload is not removed while
        // anything is still running against it.
        if let ProcessCondition::Running { pid } = self.live_process(agent_id) {
            return Err(LifecycleError::StopFailed {
                agent_id: agent_id.to_owned(),
                pid,
            });
        }

        self.begin(agent_id, state, AgentLifecycleState::Uninstalling)?;
        match self.store.uninstall(agent_id) {
            Ok(_) => {
                self.finish(
                    agent_id,
                    AgentLifecycleState::Uninstalling,
                    AgentLifecycleState::NotInstalled,
                )?;
                self.records().remove(agent_id);
                self.status(agent_id, external_candidates)
            }
            Err(error) => {
                // A refused uninstall leaves the payload intact, which is the
                // engine's fail-closed behaviour, not a broken install.
                self.finish(
                    agent_id,
                    AgentLifecycleState::Uninstalling,
                    AgentLifecycleState::Installed,
                )?;
                Err(LifecycleError::Payload(error))
            }
        }
    }

    /// Stop every running agent. Called on app shutdown.
    ///
    /// A graceful stop that leaves a child alive is escalated before moving on
    /// to the next agent. Failures are still collected so one unkillable child
    /// does not prevent cleanup of the rest.
    pub fn shutdown(&self) -> Vec<LifecycleError> {
        let agent_ids = self.records().keys().cloned().collect::<Vec<_>>();
        let mut errors = Vec::new();
        for agent_id in agent_ids {
            if let Err(error) = self.shutdown_agent(&agent_id) {
                errors.push(error);
            }
        }
        errors
    }

    fn shutdown_agent(&self, agent_id: &str) -> Result<AgentStatus, LifecycleError> {
        let lock = self.operation_lock(agent_id);
        let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
        match self.stop_locked(agent_id, ShutdownReason::AppShutdown, &[]) {
            Ok(status) => Ok(status),
            Err(LifecycleError::StopFailed { pid, .. }) => {
                let _force_requested = self.supervisor.force_stop(pid, ShutdownReason::AppShutdown);
                if self.supervisor.is_running(pid) {
                    return Err(LifecycleError::StopFailed {
                        agent_id: agent_id.to_owned(),
                        pid,
                    });
                }
                let mut records = self.records();
                if let Some(record) = records.get_mut(agent_id) {
                    if record.process.is_some_and(|process| process.pid == pid) {
                        record.process = None;
                    }
                }
                drop(records);
                self.status(agent_id, &[])
            }
            Err(error) => Err(error),
        }
    }

    /// Stop agents idle beyond the configured timeout.
    ///
    /// Opt-in: with no timeout configured this does nothing, however long an
    /// agent has been sitting there.
    pub fn sweep_idle(&self) -> Vec<LifecycleError> {
        let Some(timeout) = self.idle_timeout else {
            return Vec::new();
        };
        let idle = self
            .records()
            .iter()
            .filter(|(_, record)| {
                record.process.is_some() && record.last_activity.elapsed() >= timeout
            })
            .map(|(agent_id, _)| agent_id.clone())
            .collect::<Vec<_>>();
        idle.into_iter()
            .filter_map(|agent_id| self.stop(&agent_id, ShutdownReason::UserStopped, &[]).err())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_payload::{source_digest, PayloadShape};
    use std::collections::HashSet;
    use std::fs;

    #[test]
    fn every_state_pair_matches_the_locked_transition_matrix() {
        let expected = LEGAL_TRANSITIONS.into_iter().collect::<HashSet<_>>();
        assert_eq!(
            expected.len(),
            LEGAL_TRANSITIONS.len(),
            "legal transitions must be unique"
        );

        let mut accepted = 0;
        for from in AgentLifecycleState::ALL {
            for to in AgentLifecycleState::ALL {
                let should_accept = expected.contains(&(from, to));
                assert_eq!(
                    validate_transition(from, to).is_ok(),
                    should_accept,
                    "unexpected transition verdict for {from} -> {to}"
                );
                if should_accept {
                    accepted += 1;
                }
            }
        }
        assert_eq!(accepted, LEGAL_TRANSITIONS.len());
        assert_eq!(
            AgentLifecycleState::ALL.len() * AgentLifecycleState::ALL.len(),
            100,
            "the matrix must cover every ordered pair of the ten states"
        );
    }

    #[test]
    fn wire_strings_round_trip_and_reject_unknown_states() {
        let mut seen = HashSet::new();
        for state in AgentLifecycleState::ALL {
            assert!(seen.insert(state.as_str()), "duplicate wire string");
            assert_eq!(
                state.as_str().parse::<AgentLifecycleState>().unwrap(),
                state
            );
            assert_eq!(state.to_string(), state.as_str());
        }
        assert_eq!(
            "ready".parse::<AgentLifecycleState>().unwrap(),
            AgentLifecycleState::Ready
        );
        let error = "Ready".parse::<AgentLifecycleState>().unwrap_err();
        assert_eq!(error, ParseAgentLifecycleStateError("Ready".into()));
        assert!(error.to_string().contains("unknown agent lifecycle state"));
        assert!("".parse::<AgentLifecycleState>().is_err());
    }

    #[test]
    fn running_cannot_reach_uninstalling_without_stopping() {
        let direct = validate_transition(
            AgentLifecycleState::Running,
            AgentLifecycleState::Uninstalling,
        )
        .unwrap_err();
        assert_eq!(direct.from, AgentLifecycleState::Running);
        assert_eq!(direct.to, AgentLifecycleState::Uninstalling);
        assert!(direct
            .to_string()
            .contains("illegal agent lifecycle transition: running -> uninstalling"));

        let mut lifecycle = AgentLifecycle::new(AgentLifecycleState::Running);
        assert!(lifecycle
            .transition_to(AgentLifecycleState::Uninstalling)
            .is_err());
        assert_eq!(
            lifecycle.state(),
            AgentLifecycleState::Running,
            "a rejected transition must not mutate state"
        );
        lifecycle
            .transition_to(AgentLifecycleState::Stopping)
            .unwrap();
        lifecycle
            .transition_to(AgentLifecycleState::Uninstalling)
            .unwrap();
    }

    #[test]
    fn no_state_that_may_hold_a_process_reaches_uninstalling_directly() {
        // `stopping` is the one exception: reaching it is the act of reaping.
        for from in AgentLifecycleState::ALL
            .into_iter()
            .filter(|state| state.may_have_process() && *state != AgentLifecycleState::Stopping)
        {
            assert!(
                validate_transition(from, AgentLifecycleState::Uninstalling).is_err(),
                "{from} must stop before uninstalling"
            );
        }
    }

    #[test]
    fn external_runtimes_have_no_path_into_uninstalling() {
        assert!(!AgentLifecycleState::External.is_bridge_owned());
        assert!(validate_transition(
            AgentLifecycleState::External,
            AgentLifecycleState::Uninstalling
        )
        .is_err());
        // Nor by way of any state Bridge does not own the payload in.
        for from in AgentLifecycleState::ALL
            .into_iter()
            .filter(|state| !state.is_bridge_owned())
        {
            assert!(
                validate_transition(from, AgentLifecycleState::Uninstalling).is_err(),
                "{from} does not own a payload and must not begin uninstalling"
            );
        }
    }

    #[test]
    fn repair_and_retry_paths_are_explicit() {
        let mut lifecycle = AgentLifecycle::new(AgentLifecycleState::Repairable);
        lifecycle
            .transition_to(AgentLifecycleState::Installing)
            .unwrap();
        lifecycle
            .transition_to(AgentLifecycleState::Installed)
            .unwrap();

        let mut broken = AgentLifecycle::new(AgentLifecycleState::Broken);
        assert!(
            broken.transition_to(AgentLifecycleState::Ready).is_err(),
            "a broken agent must be reinstalled or repaired, never promoted straight to ready"
        );
        broken
            .transition_to(AgentLifecycleState::Installing)
            .unwrap();
    }

    fn installed_payload() -> PayloadCondition {
        PayloadCondition::Installed {
            entrypoint: PathBuf::from("/managed/agents/a/installations/i/payload/bin/agent"),
        }
    }

    fn external() -> Option<ExternalRuntime> {
        Some(ExternalRuntime {
            candidate: PathBuf::from("/usr/local/bin/agent"),
        })
    }

    #[test]
    fn settled_state_separates_payload_readiness_and_process() {
        let cases: [(LifecycleObservation, AgentLifecycleState); 9] = [
            (
                LifecycleObservation::absent(),
                AgentLifecycleState::NotInstalled,
            ),
            (
                LifecycleObservation {
                    external: external(),
                    ..LifecycleObservation::absent()
                },
                AgentLifecycleState::External,
            ),
            (
                LifecycleObservation {
                    payload: installed_payload(),
                    ..LifecycleObservation::absent()
                },
                AgentLifecycleState::Installed,
            ),
            (
                LifecycleObservation {
                    payload: installed_payload(),
                    readiness: ReadinessOutcome::Ready {
                        version: Some("1.2.3".into()),
                    },
                    ..LifecycleObservation::absent()
                },
                AgentLifecycleState::Ready,
            ),
            (
                LifecycleObservation {
                    payload: installed_payload(),
                    readiness: ReadinessOutcome::Unavailable {
                        reason: "Node.js 18+ is required to run Claude models".into(),
                    },
                    ..LifecycleObservation::absent()
                },
                AgentLifecycleState::Broken,
            ),
            (
                LifecycleObservation {
                    payload: installed_payload(),
                    readiness: ReadinessOutcome::Ready { version: None },
                    process: ProcessCondition::Running { pid: 4321 },
                    ..LifecycleObservation::absent()
                },
                AgentLifecycleState::Running,
            ),
            // Drift outranks readiness: an installed-but-unexecutable payload is
            // repairable, never ready. This is the #165 EntrypointNotExecutable
            // reason arriving through PayloadCondition::from_status.
            (
                LifecycleObservation {
                    payload: PayloadCondition::Repairable {
                        reason: RepairReason::EntrypointNotExecutable,
                    },
                    readiness: ReadinessOutcome::Ready { version: None },
                    ..LifecycleObservation::absent()
                },
                AgentLifecycleState::Repairable,
            ),
            // A managed payload outranks a discoverable external runtime.
            (
                LifecycleObservation {
                    payload: installed_payload(),
                    external: external(),
                    readiness: ReadinessOutcome::Ready { version: None },
                    ..LifecycleObservation::absent()
                },
                AgentLifecycleState::Ready,
            ),
            // Liveness outranks the payload condition, because `stopping` is only
            // reachable from `running`: resolving a live process to anything else
            // would leave it in a state Bridge cannot legally stop it from.
            (
                LifecycleObservation {
                    payload: PayloadCondition::Absent,
                    process: ProcessCondition::Running { pid: 99 },
                    ..LifecycleObservation::absent()
                },
                AgentLifecycleState::Running,
            ),
        ];

        for (observation, expected) in cases {
            assert_eq!(
                observation.settled_state(),
                expected,
                "unexpected settled state for {observation:?}"
            );
        }

        // And that last case really is recoverable rather than a dead end.
        let mut stranded = AgentLifecycle::new(AgentLifecycleState::Running);
        stranded
            .transition_to(AgentLifecycleState::Stopping)
            .unwrap();
        stranded
            .transition_to(AgentLifecycleState::Uninstalling)
            .unwrap();
    }

    #[test]
    fn settled_state_never_infers_an_in_flight_operation() {
        // installing/stopping/uninstalling belong to the coordinator driving an
        // operation and must never be produced by observing the world.
        for payload in [
            PayloadCondition::Absent,
            installed_payload(),
            PayloadCondition::Repairable {
                reason: RepairReason::IntegrityDrift,
            },
        ] {
            for readiness in [
                ReadinessOutcome::NotProbed,
                ReadinessOutcome::Ready { version: None },
                ReadinessOutcome::Unavailable { reason: "x".into() },
                ReadinessOutcome::VendorBlocked {
                    vendor_message: "y".into(),
                },
            ] {
                for process in [ProcessCondition::None, ProcessCondition::Running { pid: 1 }] {
                    for external in [None, external()] {
                        let state = LifecycleObservation {
                            payload: payload.clone(),
                            external,
                            readiness: readiness.clone(),
                            process,
                        }
                        .settled_state();
                        assert!(
                            !matches!(
                                state,
                                AgentLifecycleState::Installing
                                    | AgentLifecycleState::Stopping
                                    | AgentLifecycleState::Uninstalling
                            ),
                            "observation must not infer the in-flight state {state}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn vendor_auth_block_keeps_the_agent_installed_and_the_message_intact() {
        let vendor_message =
            "Not logged in. Run `codex login` to authenticate with your ChatGPT account.";
        let readiness = ReadinessOutcome::VendorBlocked {
            vendor_message: vendor_message.into(),
        };
        let observation = LifecycleObservation {
            payload: installed_payload(),
            readiness: readiness.clone(),
            ..LifecycleObservation::absent()
        };

        assert_eq!(
            observation.settled_state(),
            AgentLifecycleState::Installed,
            "a vendor auth prompt is not a Bridge failure and not readiness"
        );
        assert_ne!(observation.settled_state(), AgentLifecycleState::Broken);
        assert!(readiness.is_vendor_owned());
        assert_eq!(readiness.message(), Some(vendor_message));

        // A Bridge-side unavailability is the opposite: not vendor-owned.
        let bridge_side = ReadinessOutcome::Unavailable {
            reason: "Node.js 18+ is required".into(),
        };
        assert!(!bridge_side.is_vendor_owned());
        assert!(!ReadinessOutcome::Ready { version: None }.is_vendor_owned());
        assert_eq!(ReadinessOutcome::Ready { version: None }.message(), None);
    }

    #[test]
    fn payload_conditions_map_from_the_engine_without_reinterpretation() {
        assert_eq!(
            PayloadCondition::from_status(ManagedPayloadStatus::NotInstalled),
            PayloadCondition::Absent
        );
        for reason in [
            RepairReason::CorruptActiveReceipt,
            RepairReason::ActiveReceiptMismatch,
            RepairReason::MissingInstallation,
            RepairReason::CorruptEmbeddedReceipt,
            RepairReason::ReceiptChainMismatch,
            RepairReason::MissingPayload,
            RepairReason::MissingEntrypoint,
            RepairReason::EntrypointNotExecutable,
            RepairReason::IntegrityDrift,
            RepairReason::UnsafeManagedPath,
        ] {
            let condition =
                PayloadCondition::from_status(ManagedPayloadStatus::Repairable { reason });
            assert_eq!(
                condition,
                PayloadCondition::Repairable { reason },
                "repair reasons must survive the reduction unchanged"
            );
            assert_eq!(
                LifecycleObservation {
                    payload: condition,
                    ..LifecycleObservation::absent()
                }
                .settled_state(),
                AgentLifecycleState::Repairable
            );
        }
    }

    #[test]
    fn failure_budget_bounds_crash_loops_and_redacts_context() {
        let mut budget = FailureBudget::new(3);
        assert!(budget.may_retry() && !budget.is_exhausted());
        assert_eq!(budget.last_failure(), None);

        for attempt in 1..=3 {
            let recorded = budget.record_failure(Some("exit status: 1")).clone();
            assert_eq!(recorded.attempt, attempt);
            assert_eq!(budget.consecutive(), attempt);
        }
        assert!(
            budget.is_exhausted(),
            "three failures must exhaust a budget of three"
        );
        assert!(
            !budget.may_retry(),
            "Bridge must stop relaunching once the budget is spent"
        );

        // Counting saturates rather than wrapping back into a retryable state.
        for _ in 0..5 {
            budget.record_failure(None);
        }
        assert!(budget.is_exhausted());
        assert_eq!(
            budget.last_failure().unwrap().context,
            "no failure context reported"
        );

        // A working run clears the streak but keeps the last failure to show.
        budget.record_success();
        assert!(budget.may_retry() && !budget.is_exhausted());
        assert_eq!(budget.consecutive(), 0);
        assert!(budget.last_failure().is_some());

        // An operator retry is the only thing that clears the record too.
        budget.reset();
        assert_eq!(budget.last_failure(), None);
        assert_eq!(FailureBudget::default().max_consecutive(), 3);
    }

    #[test]
    fn retained_failure_context_is_redacted_and_bounded() {
        let mut budget = FailureBudget::default();
        let leaky = "Provider process exit status: 1. Stderr tail:\n\
             auth failed for sk-ant-api03-ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ\n\
             OPENAI_API_KEY=sk-proj-YYYYYYYYYYYYYYYYYYYYYYYYYY\n\
             Authorization: Bearer abcdefghijklmnopqrstuvwxyz012345";
        let context = budget.record_failure(Some(leaky)).context.clone();

        assert!(
            !context.contains("sk-ant-api03-ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ"),
            "an Anthropic key must not survive into lifecycle state: {context}"
        );
        assert!(!context.contains("sk-proj-YYYYYYYYYYYYYYYYYYYYYYYYYY"));
        assert!(!context.contains("abcdefghijklmnopqrstuvwxyz012345"));
        assert!(
            context.contains("exit status: 1"),
            "the useful part of the context must survive: {context}"
        );

        // Bounded, and truncated on a character boundary rather than mid-glyph.
        let mut long = FailureBudget::default();
        let oversized = format!("{}é", "s".repeat(FAILURE_CONTEXT_MAX_BYTES));
        let bounded = long.record_failure(Some(&oversized)).context.clone();
        assert!(bounded.len() <= FAILURE_CONTEXT_MAX_BYTES + '…'.len_utf8());
        assert!(bounded.ends_with('…'));
        assert!(std::str::from_utf8(bounded.as_bytes()).is_ok());
    }

    // ---- coordinator fakes -------------------------------------------------
    //
    // No test spawns a vendor process or touches the network. The payload store
    // is real, so the lifecycle is proven against the #165 engine rather than a
    // mock of it.

    struct FakeProbe {
        outcome: Mutex<ReadinessOutcome>,
    }

    impl FakeProbe {
        fn ready() -> Arc<Self> {
            Arc::new(Self {
                outcome: Mutex::new(ReadinessOutcome::Ready {
                    version: Some("1.0.0".into()),
                }),
            })
        }
        fn set(&self, outcome: ReadinessOutcome) {
            *self.outcome.lock().unwrap() = outcome;
        }
    }

    impl ReadinessProbe for FakeProbe {
        fn probe(&self, _agent_id: &str, _entrypoint: &Path) -> ReadinessOutcome {
            self.outcome.lock().unwrap().clone()
        }
    }

    struct RecordingSupervisor {
        log: Mutex<Vec<String>>,
        running: Mutex<HashSet<u32>>,
        next_pid: Mutex<u32>,
        launch_error: Mutex<Option<String>>,
        refuse_stop: Mutex<bool>,
        acknowledge_stop_without_stopping: Mutex<bool>,
        /// Checked at stop time to prove the payload had not been removed yet.
        watched_payload: Mutex<Option<PathBuf>>,
        /// When held shut, `launch` blocks inside the call. Lets a test park a
        /// launch mid-flight and run a removal to completion underneath it.
        launch_gate_open: Mutex<bool>,
        launch_gated: Mutex<bool>,
    }

    impl RecordingSupervisor {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                log: Mutex::new(Vec::new()),
                running: Mutex::new(HashSet::new()),
                next_pid: Mutex::new(1000),
                launch_error: Mutex::new(None),
                refuse_stop: Mutex::new(false),
                acknowledge_stop_without_stopping: Mutex::new(false),
                watched_payload: Mutex::new(None),
                launch_gate_open: Mutex::new(true),
                launch_gated: Mutex::new(false),
            })
        }
        fn log(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
        fn watch(&self, path: PathBuf) {
            *self.watched_payload.lock().unwrap() = Some(path);
        }
        fn fail_launch_with(&self, context: &str) {
            *self.launch_error.lock().unwrap() = Some(context.to_owned());
        }
        fn allow_launch(&self) {
            *self.launch_error.lock().unwrap() = None;
        }
        fn refuse_stop(&self, refuse: bool) {
            *self.refuse_stop.lock().unwrap() = refuse;
        }
        fn acknowledge_stop_without_stopping(&self, acknowledge: bool) {
            *self.acknowledge_stop_without_stopping.lock().unwrap() = acknowledge;
        }
        fn exit_without_notifying(&self, pid: u32) {
            self.running.lock().unwrap().remove(&pid);
        }
        /// Hold every subsequent launch inside the call until `open_launch_gate`.
        fn gate_launches(&self) {
            *self.launch_gate_open.lock().unwrap() = false;
        }
        fn open_launch_gate(&self) {
            *self.launch_gate_open.lock().unwrap() = true;
        }
        /// Did a launch actually reach the gate, i.e. get past every guard?
        fn launch_reached_gate(&self) -> bool {
            *self.launch_gated.lock().unwrap()
        }
    }

    impl ProcessSupervisor for RecordingSupervisor {
        fn launch(&self, agent_id: &str, _entrypoint: &Path) -> Result<LaunchedProcess, String> {
            if let Some(error) = self.launch_error.lock().unwrap().clone() {
                self.log
                    .lock()
                    .unwrap()
                    .push(format!("launch-failed:{agent_id}"));
                return Err(error);
            }
            if !*self.launch_gate_open.lock().unwrap() {
                *self.launch_gated.lock().unwrap() = true;
                // Bounded so a regression fails the assertion rather than hanging.
                for _ in 0..400 {
                    if *self.launch_gate_open.lock().unwrap() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            let mut next = self.next_pid.lock().unwrap();
            *next += 1;
            let pid = *next;
            self.running.lock().unwrap().insert(pid);
            self.log
                .lock()
                .unwrap()
                .push(format!("launch:{agent_id}:{pid}"));
            Ok(LaunchedProcess { pid })
        }

        fn is_running(&self, pid: u32) -> bool {
            self.running.lock().unwrap().contains(&pid)
        }

        fn stop(&self, pid: u32, reason: ShutdownReason) -> bool {
            let payload_present = self
                .watched_payload
                .lock()
                .unwrap()
                .as_ref()
                .map(|path| path.exists());
            self.log.lock().unwrap().push(format!(
                "stop:{pid}:{}:payload_present={payload_present:?}",
                reason.as_str()
            ));
            if *self.refuse_stop.lock().unwrap() {
                return false;
            }
            if *self.acknowledge_stop_without_stopping.lock().unwrap() {
                return true;
            }
            self.running.lock().unwrap().remove(&pid);
            true
        }

        fn force_stop(&self, pid: u32, reason: ShutdownReason) -> bool {
            self.log
                .lock()
                .unwrap()
                .push(format!("force-stop:{pid}:{}", reason.as_str()));
            self.running.lock().unwrap().remove(&pid);
            true
        }
    }

    fn managed_recipe(fixture: &Path, agent_id: &str) -> PayloadRecipe {
        let source = fixture.join(format!("{agent_id}-bin"));
        fs::write(&source, format!("fixture runtime for {agent_id}")).unwrap();
        let entrypoint = PathBuf::from("bin/agent");
        PayloadRecipe {
            agent_id: agent_id.into(),
            version: "1.0.0".into(),
            platform: "darwin-aarch64".into(),
            source: format!("fixture://{agent_id}"),
            expected_sha256: source_digest(&source, PayloadShape::File, &entrypoint).unwrap(),
            source_path: source,
            shape: PayloadShape::File,
            entrypoint,
        }
    }

    struct Harness {
        _fixture: tempfile::TempDir,
        fixture_path: PathBuf,
        coordinator: AgentLifecycleCoordinator,
        probe: Arc<FakeProbe>,
        supervisor: Arc<RecordingSupervisor>,
    }

    fn new_harness(idle_timeout: Option<Duration>) -> Harness {
        let fixture = tempfile::tempdir().unwrap();
        let fixture_path = fixture.path().to_path_buf();
        let probe = FakeProbe::ready();
        let supervisor = RecordingSupervisor::new();
        let coordinator = AgentLifecycleCoordinator::new(
            ManagedPayloadStore::new(fixture_path.join("managed")),
            probe.clone(),
            supervisor.clone(),
            idle_timeout,
        );
        Harness {
            _fixture: fixture,
            fixture_path,
            coordinator,
            probe,
            supervisor,
        }
    }

    const NO_CANDIDATES: &[PathBuf] = &[];

    fn go() -> AtomicBool {
        AtomicBool::new(false)
    }

    #[test]
    fn spawn_on_use_launches_only_from_ready() {
        let harness = new_harness(None);
        let recipe = managed_recipe(&harness.fixture_path, "fixture-agent");

        // not_installed: refused, and nothing was launched.
        let error = harness
            .coordinator
            .ensure_running("fixture-agent", NO_CANDIDATES)
            .unwrap_err();
        assert!(matches!(
            error,
            LifecycleError::NotReadyToLaunch {
                state: AgentLifecycleState::NotInstalled,
                ..
            }
        ));
        assert!(harness.supervisor.log().is_empty());

        // installed but readiness not proven: still refused.
        harness.probe.set(ReadinessOutcome::VendorBlocked {
            vendor_message: "Run `codex login` first.".into(),
        });
        harness
            .coordinator
            .install(&recipe, NO_CANDIDATES, &go())
            .unwrap();
        let error = harness
            .coordinator
            .ensure_running("fixture-agent", NO_CANDIDATES)
            .unwrap_err();
        assert!(
            matches!(
                error,
                LifecycleError::NotReadyToLaunch {
                    state: AgentLifecycleState::Installed,
                    ..
                }
            ),
            "a vendor-blocked agent is installed, not launchable: {error}"
        );
        assert!(
            harness.supervisor.log().is_empty(),
            "no launch was attempted"
        );

        // ready: launches.
        harness.probe.set(ReadinessOutcome::Ready { version: None });
        let status = harness
            .coordinator
            .ensure_running("fixture-agent", NO_CANDIDATES)
            .unwrap();
        assert_eq!(status.state, AgentLifecycleState::Running);
        assert!(status.process_id.is_some());
        assert_eq!(harness.supervisor.log().len(), 1);

        // Already running: idempotent, no second process.
        let again = harness
            .coordinator
            .ensure_running("fixture-agent", NO_CANDIDATES)
            .unwrap();
        assert_eq!(again.process_id, status.process_id);
        assert_eq!(harness.supervisor.log().len(), 1);
    }

    #[test]
    fn uninstall_stops_a_running_process_before_removing_the_payload() {
        let harness = new_harness(None);
        let recipe = managed_recipe(&harness.fixture_path, "fixture-agent");
        let installed = harness
            .coordinator
            .install(&recipe, NO_CANDIDATES, &go())
            .unwrap();
        assert_eq!(installed.state, AgentLifecycleState::Ready);

        let receipt = read_active_receipt(harness.coordinator.store(), "fixture-agent");
        let installation = harness
            .coordinator
            .store()
            .root()
            .join(&receipt.owned_paths[0]);
        harness.supervisor.watch(installation.clone());

        harness
            .coordinator
            .ensure_running("fixture-agent", NO_CANDIDATES)
            .unwrap();
        assert!(installation.is_dir());

        let status = harness
            .coordinator
            .uninstall("fixture-agent", NO_CANDIDATES)
            .unwrap();
        assert_eq!(status.state, AgentLifecycleState::NotInstalled);
        assert!(!installation.exists(), "the payload must be gone");

        let log = harness.supervisor.log();
        let stop = log
            .iter()
            .find(|entry| entry.starts_with("stop:"))
            .expect("uninstall must stop the process");
        assert!(
            stop.contains("payload_present=Some(true)"),
            "the process must be stopped while the payload is still on disk, got {stop}"
        );
        assert!(
            log.iter().position(|e| e.starts_with("launch:")).unwrap()
                < log.iter().position(|e| e.starts_with("stop:")).unwrap()
        );
    }

    #[test]
    fn uninstall_refuses_while_a_process_will_not_die() {
        let harness = new_harness(None);
        let recipe = managed_recipe(&harness.fixture_path, "fixture-agent");
        harness
            .coordinator
            .install(&recipe, NO_CANDIDATES, &go())
            .unwrap();
        let receipt = read_active_receipt(harness.coordinator.store(), "fixture-agent");
        let installation = harness
            .coordinator
            .store()
            .root()
            .join(&receipt.owned_paths[0]);
        harness
            .coordinator
            .ensure_running("fixture-agent", NO_CANDIDATES)
            .unwrap();

        harness.supervisor.refuse_stop(true);
        let error = harness
            .coordinator
            .uninstall("fixture-agent", NO_CANDIDATES)
            .unwrap_err();
        assert!(
            matches!(error, LifecycleError::StopFailed { .. }),
            "{error}"
        );
        assert!(
            installation.is_dir(),
            "a payload must never be removed while its process is still alive"
        );
    }

    #[test]
    fn uninstall_confirms_a_stopped_pid_is_no_longer_live() {
        let harness = new_harness(None);
        let recipe = managed_recipe(&harness.fixture_path, "fixture-agent");
        harness
            .coordinator
            .install(&recipe, NO_CANDIDATES, &go())
            .unwrap();
        let receipt = read_active_receipt(harness.coordinator.store(), "fixture-agent");
        let installation = harness
            .coordinator
            .store()
            .root()
            .join(&receipt.owned_paths[0]);
        let running = harness
            .coordinator
            .ensure_running("fixture-agent", NO_CANDIDATES)
            .unwrap();
        let pid = running.process_id.expect("launch returns a pid");

        // A supervisor may acknowledge the stop request before the process has
        // actually reaped. That acknowledgement alone cannot authorize removal.
        harness.supervisor.acknowledge_stop_without_stopping(true);
        let error = harness
            .coordinator
            .uninstall("fixture-agent", NO_CANDIDATES)
            .unwrap_err();
        assert!(matches!(error, LifecycleError::StopFailed { pid: failed, .. } if failed == pid));
        assert!(
            installation.is_dir(),
            "a payload must remain while its acknowledged-but-live process exists"
        );
        assert_eq!(
            harness
                .coordinator
                .status("fixture-agent", NO_CANDIDATES)
                .unwrap()
                .process_id,
            Some(pid),
            "the coordinator must keep tracking an unconfirmed stop"
        );
    }

    #[test]
    fn external_runtimes_cannot_be_uninstalled() {
        let harness = new_harness(None);
        let external = harness.fixture_path.join("user-installed-agent");
        fs::write(&external, b"user's own runtime").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&external, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let candidates = vec![external.clone()];

        let status = harness
            .coordinator
            .status("fixture-agent", &candidates)
            .unwrap();
        assert_eq!(status.state, AgentLifecycleState::External);

        let error = harness
            .coordinator
            .uninstall("fixture-agent", &candidates)
            .unwrap_err();
        match &error {
            LifecycleError::ExternalRuntimeNotRemovable { candidate, .. } => {
                assert_eq!(candidate, &external);
            }
            other => panic!("expected a distinct external refusal, got {other}"),
        }
        assert!(
            error.to_string().contains("holds no receipt"),
            "the refusal must say why: {error}"
        );
        assert!(external.exists(), "the user's runtime must be untouched");
        assert!(
            !harness.coordinator.store().root().join("agents").exists(),
            "no receipt may be written for a runtime Bridge does not own"
        );
    }

    #[test]
    fn cancelling_an_install_returns_to_not_installed() {
        // Cancelled before staging.
        let harness = new_harness(None);
        let recipe = managed_recipe(&harness.fixture_path, "fixture-agent");
        let cancelled = AtomicBool::new(true);
        let status = harness
            .coordinator
            .install(&recipe, NO_CANDIDATES, &cancelled)
            .unwrap();
        assert_eq!(status.state, AgentLifecycleState::NotInstalled);
        assert!(!harness
            .coordinator
            .store()
            .root()
            .join("agents/fixture-agent/active.json")
            .exists());

        // Cancelled after the payload landed: rolled back, no active receipt.
        let late = new_harness(None);
        let recipe = managed_recipe(&late.fixture_path, "fixture-agent");
        let flag = LateCancel::new();
        let status = late
            .coordinator
            .install(&recipe, NO_CANDIDATES, &flag)
            .unwrap();
        assert_eq!(
            flag.polls(),
            2,
            "cancellation must be polled before staging and again after promotion"
        );
        assert_eq!(status.state, AgentLifecycleState::NotInstalled);
        assert!(!late
            .coordinator
            .store()
            .root()
            .join("agents/fixture-agent/active.json")
            .exists());
        assert!(!late
            .coordinator
            .store()
            .root()
            .join("agents/fixture-agent/installations")
            .read_dir()
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(false));
    }

    #[test]
    fn failed_install_returns_an_error_and_never_strands_installing() {
        let harness = new_harness(None);
        let recipe = managed_recipe(&harness.fixture_path, "fixture-agent");
        fs::write(&recipe.source_path, b"tampered after recipe resolution").unwrap();

        let error = harness
            .coordinator
            .install(&recipe, NO_CANDIDATES, &go())
            .unwrap_err();
        assert!(matches!(
            error,
            LifecycleError::Payload(BridgeError::Invalid(_))
        ));
        assert_eq!(
            harness
                .coordinator
                .status("fixture-agent", NO_CANDIDATES)
                .unwrap()
                .state,
            AgentLifecycleState::NotInstalled,
            "a failed install must not report a synthetic in-flight state"
        );

        // A corrected recipe can run immediately, proving the failed operation
        // cleared its in-flight state rather than wedging all future work.
        let corrected = managed_recipe(&harness.fixture_path, "fixture-agent");
        assert_eq!(
            harness
                .coordinator
                .install(&corrected, NO_CANDIDATES, &go())
                .unwrap()
                .state,
            AgentLifecycleState::Ready
        );
    }

    #[test]
    fn failed_cancellation_rollback_never_strands_installing() {
        let harness = new_harness(None);
        let recipe = managed_recipe(&harness.fixture_path, "fixture-agent");
        let cancellation = CorruptRollbackCancel::new(
            harness
                .coordinator
                .store()
                .root()
                .join("agents/fixture-agent/active.json"),
        );

        // The second cancellation poll runs after promotion. Corrupting the
        // active receipt makes the rollback correctly fail closed.
        assert!(matches!(
            harness
                .coordinator
                .install(&recipe, NO_CANDIDATES, &cancellation)
                .unwrap_err(),
            LifecycleError::Payload(_)
        ));
        assert_eq!(
            harness
                .coordinator
                .status("fixture-agent", NO_CANDIDATES)
                .unwrap()
                .state,
            AgentLifecycleState::Repairable,
            "rollback failure must expose the payload condition, not installing"
        );

        // The embedded receipt still proves ownership, so the next explicit
        // install repairs it instead of being blocked by stale coordinator state.
        assert_eq!(
            harness
                .coordinator
                .install(&recipe, NO_CANDIDATES, &go())
                .unwrap()
                .state,
            AgentLifecycleState::Ready
        );
    }

    /// Cancelled only after the first poll, so the pre-staging check passes and
    /// the post-promotion check fires — the case that needs a rollback.
    struct LateCancel {
        polls: Mutex<u32>,
    }

    impl LateCancel {
        fn new() -> Self {
            Self {
                polls: Mutex::new(0),
            }
        }
        fn polls(&self) -> u32 {
            *self.polls.lock().unwrap()
        }
    }

    impl CancellationSignal for LateCancel {
        fn is_cancelled(&self) -> bool {
            let mut polls = self.polls.lock().unwrap();
            *polls += 1;
            *polls > 1
        }
    }

    /// Cancels after promotion while making the rollback's active receipt
    /// corrupt, so the payload engine must refuse the rollback.
    struct CorruptRollbackCancel {
        polls: Mutex<u32>,
        active_receipt: PathBuf,
    }

    impl CorruptRollbackCancel {
        fn new(active_receipt: PathBuf) -> Self {
            Self {
                polls: Mutex::new(0),
                active_receipt,
            }
        }
    }

    impl CancellationSignal for CorruptRollbackCancel {
        fn is_cancelled(&self) -> bool {
            let mut polls = self.polls.lock().unwrap();
            *polls += 1;
            if *polls == 2 {
                fs::write(&self.active_receipt, b"corrupt receipt for rollback test").unwrap();
                true
            } else {
                false
            }
        }
    }

    #[test]
    fn app_shutdown_stops_every_running_agent() {
        let harness = new_harness(None);
        for agent_id in ["first-agent", "second-agent"] {
            let recipe = managed_recipe(&harness.fixture_path, agent_id);
            harness
                .coordinator
                .install(&recipe, NO_CANDIDATES, &go())
                .unwrap();
            harness
                .coordinator
                .ensure_running(agent_id, NO_CANDIDATES)
                .unwrap();
        }

        let errors = harness.coordinator.shutdown();
        assert!(errors.is_empty(), "{errors:?}");
        for agent_id in ["first-agent", "second-agent"] {
            let status = harness.coordinator.status(agent_id, NO_CANDIDATES).unwrap();
            assert!(
                !status.state.may_have_process(),
                "{agent_id} was left in {}",
                status.state
            );
            assert_eq!(status.process_id, None);
        }
        let stops = harness
            .supervisor
            .log()
            .into_iter()
            .filter(|entry| entry.starts_with("stop:"))
            .collect::<Vec<_>>();
        assert_eq!(stops.len(), 2);
        assert!(
            stops.iter().all(|entry| entry.contains("app_shutdown")),
            "every stop must be attributed to app shutdown: {stops:?}"
        );
    }

    #[test]
    fn app_shutdown_escalates_a_process_that_refuses_graceful_stop() {
        let harness = new_harness(None);
        let recipe = managed_recipe(&harness.fixture_path, "fixture-agent");
        harness
            .coordinator
            .install(&recipe, NO_CANDIDATES, &go())
            .unwrap();
        let running = harness
            .coordinator
            .ensure_running("fixture-agent", NO_CANDIDATES)
            .unwrap();
        let pid = running.process_id.expect("launch returns a pid");

        harness.supervisor.refuse_stop(true);
        assert!(
            harness.coordinator.shutdown().is_empty(),
            "shutdown must escalate instead of leaving a live child behind"
        );
        let status = harness
            .coordinator
            .status("fixture-agent", NO_CANDIDATES)
            .unwrap();
        assert!(!status.state.may_have_process());
        assert_eq!(status.process_id, None);
        assert!(
            harness
                .supervisor
                .log()
                .iter()
                .any(|entry| entry == &format!("force-stop:{pid}:app_shutdown")),
            "a refusing child must receive the shutdown escalation"
        );
    }

    #[test]
    fn idle_shutdown_is_opt_in_and_bounded() {
        // Not configured: a running agent is left alone however long it sits.
        let never = new_harness(None);
        let recipe = managed_recipe(&never.fixture_path, "fixture-agent");
        never
            .coordinator
            .install(&recipe, NO_CANDIDATES, &go())
            .unwrap();
        never
            .coordinator
            .ensure_running("fixture-agent", NO_CANDIDATES)
            .unwrap();
        assert!(never.coordinator.sweep_idle().is_empty());
        assert_eq!(
            never
                .coordinator
                .status("fixture-agent", NO_CANDIDATES)
                .unwrap()
                .state,
            AgentLifecycleState::Running,
            "with no idle timeout nothing is reaped"
        );

        // Configured but not yet reached: still left alone.
        let patient = new_harness(Some(Duration::from_secs(3600)));
        let recipe = managed_recipe(&patient.fixture_path, "fixture-agent");
        patient
            .coordinator
            .install(&recipe, NO_CANDIDATES, &go())
            .unwrap();
        patient
            .coordinator
            .ensure_running("fixture-agent", NO_CANDIDATES)
            .unwrap();
        assert!(patient.coordinator.sweep_idle().is_empty());
        assert_eq!(
            patient
                .coordinator
                .status("fixture-agent", NO_CANDIDATES)
                .unwrap()
                .state,
            AgentLifecycleState::Running
        );

        // Reached: stopped, and back to ready rather than gone.
        let eager = new_harness(Some(Duration::ZERO));
        let recipe = managed_recipe(&eager.fixture_path, "fixture-agent");
        eager
            .coordinator
            .install(&recipe, NO_CANDIDATES, &go())
            .unwrap();
        eager
            .coordinator
            .ensure_running("fixture-agent", NO_CANDIDATES)
            .unwrap();
        assert!(eager.coordinator.sweep_idle().is_empty());
        let status = eager
            .coordinator
            .status("fixture-agent", NO_CANDIDATES)
            .unwrap();
        assert_eq!(status.state, AgentLifecycleState::Ready);
        assert_eq!(status.process_id, None);
        assert!(eager
            .supervisor
            .log()
            .iter()
            .any(|entry| entry.starts_with("stop:")));
    }

    #[test]
    fn a_crash_loop_stops_relaunching_and_keeps_redacted_context() {
        let harness = new_harness(None);
        let recipe = managed_recipe(&harness.fixture_path, "fixture-agent");
        harness
            .coordinator
            .install(&recipe, NO_CANDIDATES, &go())
            .unwrap();
        harness.supervisor.fail_launch_with(
            "spawn failed: ANTHROPIC_API_KEY=sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAA",
        );

        for _ in 0..DEFAULT_MAX_CONSECUTIVE_FAILURES {
            let error = harness
                .coordinator
                .ensure_running("fixture-agent", NO_CANDIDATES)
                .unwrap_err();
            match error {
                LifecycleError::LaunchFailed { context, .. } => assert!(
                    !context.contains("sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAA"),
                    "launch failure context must be redacted: {context}"
                ),
                other => panic!("expected a launch failure, got {other}"),
            }
        }

        // Budget spent: Bridge stops trying on its own, even though the payload
        // is still perfectly installable.
        harness.supervisor.allow_launch();
        let error = harness
            .coordinator
            .ensure_running("fixture-agent", NO_CANDIDATES)
            .unwrap_err();
        assert!(
            matches!(error, LifecycleError::RetryBudgetExhausted { attempts, .. }
                if attempts == DEFAULT_MAX_CONSECUTIVE_FAILURES),
            "{error}"
        );
        let status = harness
            .coordinator
            .status("fixture-agent", NO_CANDIDATES)
            .unwrap();
        assert_eq!(
            status.consecutive_failures,
            DEFAULT_MAX_CONSECUTIVE_FAILURES
        );
        assert_eq!(
            status.state,
            AgentLifecycleState::Broken,
            "an agent Bridge has stopped relaunching must not report itself ready"
        );
        // ...and it is still Bridge's to remove or reinstall from there.
        assert!(status.state.is_bridge_owned());
        assert!(validate_transition(status.state, AgentLifecycleState::Installing).is_ok());
        assert!(validate_transition(status.state, AgentLifecycleState::Uninstalling).is_ok());
        let retained = status.last_failure.expect("failure context is retained");
        assert!(!retained.context.contains("sk-ant-api03"));
        assert!(retained.context.contains("spawn failed"));

        // An explicit retry is what clears it.
        harness.coordinator.reset_retry_budget("fixture-agent");
        let status = harness
            .coordinator
            .ensure_running("fixture-agent", NO_CANDIDATES)
            .unwrap();
        assert_eq!(status.state, AgentLifecycleState::Running);
    }

    #[test]
    fn exhausted_retry_budget_does_not_recast_vendor_auth_as_bridge_failure() {
        let harness = new_harness(None);
        let recipe = managed_recipe(&harness.fixture_path, "fixture-agent");
        harness
            .coordinator
            .install(&recipe, NO_CANDIDATES, &go())
            .unwrap();
        harness
            .supervisor
            .fail_launch_with("process repeatedly crashed");
        for _ in 0..DEFAULT_MAX_CONSECUTIVE_FAILURES {
            assert!(matches!(
                harness
                    .coordinator
                    .ensure_running("fixture-agent", NO_CANDIDATES)
                    .unwrap_err(),
                LifecycleError::LaunchFailed { .. }
            ));
        }
        harness.probe.set(ReadinessOutcome::VendorBlocked {
            vendor_message: "Run `codex login` first.".into(),
        });

        let status = harness
            .coordinator
            .status("fixture-agent", NO_CANDIDATES)
            .unwrap();
        assert_eq!(
            status.state,
            AgentLifecycleState::Installed,
            "the vendor login state must outrank the retry breaker"
        );
        assert_eq!(status.vendor_message(), Some("Run `codex login` first."));
        assert_eq!(
            status.consecutive_failures,
            DEFAULT_MAX_CONSECUTIVE_FAILURES
        );
    }

    #[test]
    fn a_full_lifecycle_walk_ends_with_no_receipt() {
        let harness = new_harness(None);
        let recipe = managed_recipe(&harness.fixture_path, "fixture-agent");
        let unrelated = harness.fixture_path.join("managed/unrelated-file");

        // A user-managed runtime sits beside the managed one for the whole walk:
        // the managed payload is what Bridge launches, and the user's own copy
        // must still be there at the end.
        let external = harness.fixture_path.join("user-installed-agent");
        fs::write(&external, b"user's own runtime").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&external, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let candidates = vec![external.clone()];

        assert_eq!(
            harness
                .coordinator
                .status("fixture-agent", &candidates)
                .unwrap()
                .state,
            AgentLifecycleState::External,
            "with nothing managed, the discoverable runtime is what there is"
        );
        assert_eq!(
            harness
                .coordinator
                .status("fixture-agent", NO_CANDIDATES)
                .unwrap()
                .state,
            AgentLifecycleState::NotInstalled
        );
        assert_eq!(
            harness
                .coordinator
                .install(&recipe, &candidates, &go())
                .unwrap()
                .state,
            AgentLifecycleState::Ready,
            "a managed payload outranks a discoverable external runtime"
        );
        fs::write(&unrelated, b"keep").unwrap();

        assert_eq!(
            harness
                .coordinator
                .ensure_running("fixture-agent", &candidates)
                .unwrap()
                .state,
            AgentLifecycleState::Running
        );
        assert_eq!(
            harness
                .coordinator
                .stop("fixture-agent", ShutdownReason::UserStopped, &candidates)
                .unwrap()
                .state,
            AgentLifecycleState::Ready
        );
        // Spawn-on-use works again after a clean stop.
        assert_eq!(
            harness
                .coordinator
                .ensure_running("fixture-agent", &candidates)
                .unwrap()
                .state,
            AgentLifecycleState::Running
        );
        harness
            .coordinator
            .stop("fixture-agent", ShutdownReason::UserStopped, &candidates)
            .unwrap();

        // Removing the managed payload reveals the external runtime again rather
        // than reporting nothing, and never touches the user's own copy.
        assert_eq!(
            harness
                .coordinator
                .uninstall("fixture-agent", &candidates)
                .unwrap()
                .state,
            AgentLifecycleState::External
        );
        assert!(!harness
            .coordinator
            .store()
            .root()
            .join("agents/fixture-agent/active.json")
            .exists());
        assert!(
            unrelated.exists(),
            "unrelated managed-root content survives"
        );
        assert_eq!(
            fs::read(&external).unwrap(),
            b"user's own runtime",
            "the user's runtime must be byte-identical after a managed uninstall"
        );

        // Uninstalling again converges rather than erroring — and now that only
        // the external runtime is left, it is refused rather than repeated.
        assert!(matches!(
            harness
                .coordinator
                .uninstall("fixture-agent", &candidates)
                .unwrap_err(),
            LifecycleError::ExternalRuntimeNotRemovable { .. }
        ));
        assert_eq!(
            harness
                .coordinator
                .uninstall("fixture-agent", NO_CANDIDATES)
                .unwrap()
                .state,
            AgentLifecycleState::NotInstalled
        );
        assert!(external.exists());
    }

    #[test]
    fn a_drifted_payload_is_repairable_and_not_launchable() {
        let harness = new_harness(None);
        let recipe = managed_recipe(&harness.fixture_path, "fixture-agent");
        harness
            .coordinator
            .install(&recipe, NO_CANDIDATES, &go())
            .unwrap();
        let receipt = read_active_receipt(harness.coordinator.store(), "fixture-agent");
        fs::write(
            harness.coordinator.store().root().join(&receipt.entrypoint),
            b"tampered",
        )
        .unwrap();

        let status = harness
            .coordinator
            .status("fixture-agent", NO_CANDIDATES)
            .unwrap();
        assert_eq!(status.state, AgentLifecycleState::Repairable);
        assert!(matches!(
            harness
                .coordinator
                .ensure_running("fixture-agent", NO_CANDIDATES)
                .unwrap_err(),
            LifecycleError::NotReadyToLaunch {
                state: AgentLifecycleState::Repairable,
                ..
            }
        ));
        // Repairable payloads need the receipt-proven repair path, not a fresh
        // install (which correctly refuses to overwrite drift).
        let repaired = harness
            .coordinator
            .install(&recipe, NO_CANDIDATES, &go())
            .unwrap();
        assert_eq!(repaired.state, AgentLifecycleState::Ready);
        assert_eq!(
            harness
                .coordinator
                .ensure_running("fixture-agent", NO_CANDIDATES)
                .unwrap()
                .state,
            AgentLifecycleState::Running,
            "a repaired payload is launchable again"
        );
        // A drifted payload is still Bridge's to remove.
        assert_eq!(
            harness
                .coordinator
                .uninstall("fixture-agent", NO_CANDIDATES)
                .unwrap()
                .state,
            AgentLifecycleState::NotInstalled
        );
    }

    #[test]
    fn vendor_auth_never_becomes_bridge_failure_state() {
        let harness = new_harness(None);
        let recipe = managed_recipe(&harness.fixture_path, "fixture-agent");
        let vendor_message = "Not logged in. Run `codex login`.";
        harness.probe.set(ReadinessOutcome::VendorBlocked {
            vendor_message: vendor_message.into(),
        });
        let status = harness
            .coordinator
            .install(&recipe, NO_CANDIDATES, &go())
            .unwrap();

        assert_eq!(status.state, AgentLifecycleState::Installed);
        assert_eq!(status.vendor_message(), Some(vendor_message));
        assert_eq!(
            status.last_failure, None,
            "a vendor login prompt is not a Bridge failure and must not consume the retry budget"
        );
        assert_eq!(status.consecutive_failures, 0);

        // Logging in with the vendor is all it takes; Bridge stored nothing.
        harness.probe.set(ReadinessOutcome::Ready { version: None });
        let status = harness
            .coordinator
            .status("fixture-agent", NO_CANDIDATES)
            .unwrap();
        assert_eq!(status.state, AgentLifecycleState::Ready);
        assert_eq!(status.vendor_message(), None);
    }

    #[test]
    fn a_launch_can_never_race_an_uninstall_onto_a_deleted_payload() {
        // The stopping-first rule keeps a removal off a process that is *already*
        // running. This covers the other half of the issue's requirement: a
        // launch starting while a removal runs must not end up holding an
        // entrypoint that no longer exists.
        //
        // The interleaving is forced rather than raced for. The launcher thread
        // is parked inside `launch`, the removal is run to completion underneath
        // it, and only then is the launch released — so if operations are not
        // serialized per agent, the launch returns a live process whose payload
        // is already gone. Verified to fail when the operation lock is removed.
        let harness = Arc::new(new_harness(None));
        let recipe = managed_recipe(&harness.fixture_path, "fixture-agent");
        harness
            .coordinator
            .install(&recipe, NO_CANDIDATES, &go())
            .unwrap();
        let receipt = read_active_receipt(harness.coordinator.store(), "fixture-agent");
        let installation = harness
            .coordinator
            .store()
            .root()
            .join(&receipt.owned_paths[0]);

        harness.supervisor.gate_launches();
        let launcher = {
            let harness = Arc::clone(&harness);
            std::thread::spawn(move || {
                harness
                    .coordinator
                    .ensure_running("fixture-agent", NO_CANDIDATES)
                    .map(|status| status.process_id)
            })
        };

        // Give the launcher time to get as far as it is allowed to.
        std::thread::sleep(Duration::from_millis(80));
        let removed = harness
            .coordinator
            .uninstall("fixture-agent", NO_CANDIDATES);
        harness.supervisor.open_launch_gate();
        let launched = launcher.join().unwrap();

        // Either order is safe, and serialization is what makes both safe:
        //
        //   * the launch won the agent, so the removal waited, stopped the
        //     process it found, and only then removed the payload; or
        //   * the removal won, so the launch re-observed an absent payload and
        //     was refused.
        //
        // The invariant is the same either way and is what the missing lock
        // breaks: a live process and a removed payload must never coexist.
        let payload_present = installation.exists();
        let live = harness.coordinator.live_process("fixture-agent");
        assert!(
            payload_present || matches!(live, ProcessCondition::None),
            "a process is alive with no payload behind it (launched={launched:?}, \
             removed={:?}, reached_supervisor={})",
            removed.as_ref().map(|status| status.state),
            harness.supervisor.launch_reached_gate()
        );
        if let Ok(Some(pid)) = launched {
            assert!(
                payload_present || !harness.supervisor.is_running(pid),
                "launched pid {pid} outlived its payload"
            );
        }
        // And the removal itself must have converged rather than been starved.
        assert!(
            removed.is_ok(),
            "uninstall failed: {:?}",
            removed.err().map(|error| error.to_string())
        );
        assert!(
            !payload_present,
            "uninstall reported success but the payload is still on disk"
        );
        assert!(
            matches!(live, ProcessCondition::None),
            "nothing may still be running once the payload is gone"
        );
    }

    #[test]
    fn a_process_exiting_on_its_own_returns_the_agent_to_ready() {
        let harness = new_harness(None);
        let recipe = managed_recipe(&harness.fixture_path, "fixture-agent");
        harness
            .coordinator
            .install(&recipe, NO_CANDIDATES, &go())
            .unwrap();
        let first = harness
            .coordinator
            .ensure_running("fixture-agent", NO_CANDIDATES)
            .unwrap();
        let first_pid = first.process_id.expect("launch returns a pid");

        // A clean exit: back to ready, and spawn-on-use works again.
        harness
            .coordinator
            .note_process_exit("fixture-agent", first_pid, None)
            .unwrap();
        let status = harness
            .coordinator
            .status("fixture-agent", NO_CANDIDATES)
            .unwrap();
        assert_eq!(status.state, AgentLifecycleState::Ready);
        assert_eq!(status.process_id, None);
        assert_eq!(status.consecutive_failures, 0);
        let replacement = harness
            .coordinator
            .ensure_running("fixture-agent", NO_CANDIDATES)
            .unwrap();
        assert_eq!(replacement.state, AgentLifecycleState::Running);
        let replacement_pid = replacement.process_id.expect("launch returns a pid");

        // A crash counts against the budget and keeps redacted context, but one
        // crash is not a loop: the agent is still launchable.
        harness
            .coordinator
            .note_process_exit(
                "fixture-agent",
                replacement_pid,
                Some("segfault; GITHUB_TOKEN=ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
            )
            .unwrap();
        let status = harness
            .coordinator
            .status("fixture-agent", NO_CANDIDATES)
            .unwrap();
        assert_eq!(status.consecutive_failures, 1);
        assert_eq!(status.state, AgentLifecycleState::Ready);
        let context = status.last_failure.unwrap().context;
        assert!(context.contains("segfault"));
        assert!(
            !context.contains("ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
            "a token in an exit context must not reach lifecycle state: {context}"
        );
    }

    #[test]
    fn a_delayed_exit_from_an_old_process_cannot_clear_its_replacement() {
        let harness = new_harness(None);
        let recipe = managed_recipe(&harness.fixture_path, "fixture-agent");
        harness
            .coordinator
            .install(&recipe, NO_CANDIDATES, &go())
            .unwrap();
        let first = harness
            .coordinator
            .ensure_running("fixture-agent", NO_CANDIDATES)
            .unwrap();
        let first_pid = first.process_id.expect("launch returns a pid");

        // The child has exited, but its delayed notification has not arrived.
        harness.supervisor.exit_without_notifying(first_pid);
        let replacement = harness
            .coordinator
            .ensure_running("fixture-agent", NO_CANDIDATES)
            .unwrap();
        let replacement_pid = replacement.process_id.expect("replacement launches");
        assert_ne!(replacement_pid, first_pid);

        harness
            .coordinator
            .note_process_exit("fixture-agent", first_pid, Some("stale crash notification"))
            .unwrap();
        let status = harness
            .coordinator
            .status("fixture-agent", NO_CANDIDATES)
            .unwrap();
        assert_eq!(status.state, AgentLifecycleState::Running);
        assert_eq!(status.process_id, Some(replacement_pid));
        assert_eq!(
            status.consecutive_failures, 0,
            "a stale exit must not count against the replacement process"
        );
    }

    fn read_active_receipt(
        store: &ManagedPayloadStore,
        agent_id: &str,
    ) -> crate::managed_payload::ManagedPayloadReceipt {
        let path = store
            .root()
            .join("agents")
            .join(agent_id)
            .join("active.json");
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn nothing_reaches_installed_ready_or_running_without_passing_through_install() {
        for target in [
            AgentLifecycleState::Ready,
            AgentLifecycleState::Running,
            AgentLifecycleState::Stopping,
        ] {
            assert!(
                validate_transition(AgentLifecycleState::NotInstalled, target).is_err(),
                "not_installed must not jump to {target}"
            );
        }
        assert!(validate_transition(
            AgentLifecycleState::NotInstalled,
            AgentLifecycleState::Installed
        )
        .is_err());
    }
}

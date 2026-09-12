//! The Cursor harness: finding the vendor's CLI, proving it speaks the
//! protocol, and running a session through the shared ACP client.
//!
//! **This is the first harness Bridge adopts without writing a provider
//! adapter.** Codex, Claude and OpenCode each needed a private wire format
//! learned by hand; Cursor ships an ACP server inside its own CLI, so
//! everything after the handshake — turns, streaming, permissions, cancel,
//! replay — is [`crate::acp_session`] and [`crate::acp_events`] unchanged. What
//! remains is Cursor-specific and is all that lives here: where the executable
//! is, whether it is really the one Bridge means, whether this build of it
//! speaks the protocol at all, and where a credential may and may not appear.
//!
//! **Two names, one executable, and only one of them is evidence.** The
//! installer puts `agent` on PATH; the published registry entry and the vendor's
//! own documentation use `cursor-agent`. The bare name is shared with an
//! unrelated vendor's CLI, so resolving it proves nothing on its own. Bridge
//! prefers `cursor-agent`, and when only `agent` is found it requires the
//! executable to identify itself as Cursor during the handshake before the
//! harness is offered. Discovery never installs, downloads, or modifies
//! anything: this is a vendor CLI the user chose to install, exactly like the
//! other three.
//!
//! **The protocol is confirmed, never assumed.** `acp` is a hidden subcommand —
//! it is absent from `--help`, so support cannot be established by reading help
//! output. Worse, a build that predates it does not reject it: it treats `acp`
//! as a prompt, starts an interactive terminal session, and paints control
//! sequences onto stdout while a client waits forever for a handshake. Reading
//! `--version` narrows the question and does not answer it, because the version
//! string is a bare date and hash with no name in it and no published cutover.
//! So the only sound test is the handshake itself, under a timeout, with
//! non-protocol output treated as a failed probe rather than as noise to skip
//! past. The shared client already separates the two: a stdout line that is not
//! shaped like a JSON message never reaches the parser and lands in the bounded
//! noise tail, which is what makes a terminal-painting build legible here
//! instead of merely silent.
//!
//! **A probe costs a process, so it is taken once per build.** The result is
//! cached against the path and version it was taken against. Reading adapter
//! availability is a hot path — the descriptor is rebuilt on every state read —
//! and re-probing there would spawn a child per keystroke. A version that moved
//! invalidates the cache, because the whole point of the probe is that support
//! is a property of the build.
//!
//! **Authentication stays the vendor's.** Bridge never reads, copies, or stores
//! Cursor's credentials, profile, or cookies: an existing login made with the
//! vendor's own command is used as it stands, and a session that needs one is
//! reported as needing sign-in with that command named. Where a key is
//! configured it is passed in the launched process environment and nowhere
//! else — never on the command line, where `--api-key` would put it in every
//! process listing on the machine, and never in a descriptor, an availability
//! reason, or a failure report.
//!
//! **Capabilities are read off the session.** Models and modes are whatever the
//! agent advertised when the session opened, and their identifiers are echoed
//! back byte for byte. That is not fastidiousness: Cursor's model identifiers
//! live in namespaces that differ per picker mode and match neither the
//! command-line spelling nor the display label, and sending a reconstructed one
//! fails *every* model with an invalid-params error that reads to a user like a
//! lapsed subscription. Bridge therefore never derives an identifier from a
//! label, and it never asks for the richer picker: opting into it means sending
//! a client `_meta` flag whose exact spelling is not published, and a guessed
//! key would silently select the wrong namespace. The default picker is what
//! the session reports and what Bridge offers.

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
    model::{
        AdapterDescriptor, AuthState, CapabilityTier, ModelCatalogDiagnostics, ModelCatalogSource,
        ModelLifecycle, ModelOption,
    },
    model_catalog::{self, CatalogCandidate},
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
        Arc, Mutex, RwLock,
    },
    thread,
    time::Duration,
};

pub const HARNESS_ID: &str = "cursor";
pub const HARNESS_LABEL: &str = "Cursor";

/// The vendor's published executable name, and the one the installer also puts
/// on PATH. Order is preference order, and the preference is the whole point:
/// `agent` is a name an unrelated vendor's CLI also claims.
const PUBLISHED_EXECUTABLE: &str = "cursor-agent";
const AMBIGUOUS_EXECUTABLE: &str = "agent";

/// The protocol server is a subcommand, not a flag, and global flags precede
/// it. Nothing else is ever appended: an argument vector is world-readable.
const ACP_SUBCOMMAND: &str = "acp";

/// Environment variables the vendor reads a key from, in the order it does.
/// Forwarded explicitly at the launch site rather than left to inheritance, so
/// the one place a credential may travel is stated in the code that travels it.
const KEY_VARIABLES: [&str; 2] = ["CURSOR_API_KEY", "CURSOR_AUTH_TOKEN"];

/// The vendor's own sign-in command, named in the reason a signed-out harness
/// gives. Bridge cannot perform this login and does not try to.
const SIGN_IN_COMMAND: &str = "cursor-agent login";

/// How long the probe waits for a handshake. Shorter than a session's own
/// budget on purpose: a probe is a question about the build, asked while the
/// user is waiting to see whether the harness is selectable at all, and a
/// pre-protocol build answers it by never answering.
const PROBE_TIMEOUT: Duration = Duration::from_secs(12);

/// What an agent has to say about itself before the ambiguous name counts as
/// evidence. Matched case-insensitively against the agent's own reported name
/// rather than pinned to a spelling: the string is the vendor's, and it has no
/// published stability guarantee.
const VENDOR_MARKER: &str = "cursor";

/// How often the event pump moves the session's queue onto the reader.
///
/// The shared client buffers events for a caller that drains them, which is the
/// right shape for a caller that owns a thread and the wrong one for Bridge's
/// reader contract, so this module bridges the two. The interval is short
/// enough to be invisible beside a model's own token cadence and long enough
/// that an idle session is not a busy loop.
const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(15);

/// Where the vendor's CLI was found, and under which of its two names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorExecutable {
    pub path: PathBuf,
    /// True when the name that resolved was the ambiguous one, which is what
    /// makes self-identification during the handshake load-bearing rather than
    /// decorative.
    pub ambiguous_name: bool,
    pub version: String,
}

/// Why the harness is not on offer, stated so a user can act on it.
///
/// Each variant names a different remedy — install it, upgrade it, sign in,
/// look at what it printed — and they are kept apart because collapsing them
/// into one "unavailable" is what makes a missing binary read as a billing
/// problem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CursorUnavailable {
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

impl CursorUnavailable {
    /// The sentence shown on the descriptor. Never carries a credential: every
    /// value interpolated here is a path, a version, or text that has already
    /// been through [`redact`].
    pub fn reason(&self) -> String {
        match self {
            Self::NotInstalled => format!(
                "Cursor is not installed; Bridge looked for {PUBLISHED_EXECUTABLE} and \
                 {AMBIGUOUS_EXECUTABLE} on PATH"
            ),
            Self::UnreadableVersion { path } => format!(
                "Cursor at {} did not report a version, so Bridge cannot tell whether this \
                 build speaks the agent protocol",
                path.display()
            ),
            Self::NotProtocol { version, output } => {
                let detail = output
                    .as_deref()
                    .map(|output| format!("; it printed: {output}"))
                    .unwrap_or_default();
                format!(
                    "Cursor {version} answered {ACP_SUBCOMMAND} with terminal output instead of \
                     the agent protocol, so this build cannot be driven{detail}"
                )
            }
            Self::Unidentified { version, reported } => match reported {
                Some(name) => format!(
                    "The executable found as {AMBIGUOUS_EXECUTABLE} ({version}) identified \
                     itself as {name:?}, not as Cursor; install {PUBLISHED_EXECUTABLE} so \
                     Bridge can tell the two apart"
                ),
                None => format!(
                    "The executable found as {AMBIGUOUS_EXECUTABLE} ({version}) has not \
                     identified itself as Cursor; install {PUBLISHED_EXECUTABLE} so Bridge can \
                     tell the two apart"
                ),
            },
            Self::NeedsSignIn { version } => {
                format!("Cursor {version} is installed but not signed in; run {SIGN_IN_COMMAND}")
            }
            Self::ProbeFailed { version, reason } => {
                format!("Cursor {version} could not start an agent session: {reason}")
            }
        }
    }

    /// Whether the vendor, not Bridge, is the thing that needs attention. Kept
    /// distinct from availability: a signed-out Cursor is installed and working,
    /// and reporting it as absent would send the user to the wrong place.
    const fn auth_state(&self) -> AuthState {
        match self {
            Self::NeedsSignIn { .. } => AuthState::SignedOut,
            _ => AuthState::Unknown,
        }
    }
}

/// What one completed probe established about a build.
#[derive(Debug, Clone, PartialEq)]
pub struct CursorProfile {
    pub executable: PathBuf,
    pub version: String,
    /// The agent's own name, as it reported it.
    pub agent_name: Option<String>,
    /// `session/load` — history replay. Advertised by Cursor.
    pub load_session: bool,
    /// `session/resume` — reconnecting without replay. Not advertised by
    /// Cursor, and never inferred from `load_session`: they are separate
    /// capabilities answering separate questions.
    pub resume_session: bool,
    pub additional_directories: bool,
    pub prompt_images: bool,
    /// Authentication methods the agent advertised, by the ids it used.
    pub auth_methods: Vec<String>,
    /// `initialize` proves protocol support, not an authenticated provider
    /// session. This becomes true only after a real `session/new` succeeds.
    pub session_opened: bool,
    /// Session modes, in the order advertised, with the current one first in
    /// [`Self::current_mode`].
    pub modes: Vec<String>,
    pub current_mode: Option<String>,
    /// Models the session reported, with their identifiers exactly as received.
    pub models: Vec<ModelOption>,
    pub default_model: Option<String>,
}

/// One probe, and the build it was taken against.
#[derive(Debug, Clone)]
struct CachedProbe {
    executable: PathBuf,
    version: String,
    outcome: Result<CursorProfile, CursorUnavailable>,
}

impl CachedProbe {
    /// Whether this result still describes the executable in front of us. Both
    /// halves matter: a version bump is a different build of the same install,
    /// and a different path is a different install altogether.
    fn describes(&self, executable: &CursorExecutable) -> bool {
        self.executable == executable.path && self.version == executable.version
    }
}

/// Find the vendor's CLI, preferring a Bridge-managed payload and then the name
/// that identifies it.
///
/// A managed payload outranks PATH for the reason it does in every other
/// adapter: it is the copy the user asked Bridge to install, and searching PATH
/// first would leave an installed payload unusable. It is never the ambiguous
/// name — Bridge extracted it from its own digest-pinned recipe.
///
/// `managed` and `resolve` are lookups rather than hardcoded calls so the
/// preference order can be exercised against a controlled search path;
/// production passes [`crate::binary::resolve`], which is also what finds a CLI
/// when Bridge is launched as a macOS bundle without a login shell's PATH.
fn locate_with(
    managed: &dyn Fn() -> Option<PathBuf>,
    resolve: &dyn Fn(&str) -> Option<PathBuf>,
    version_at: &dyn Fn(&Path) -> Option<String>,
) -> Result<CursorExecutable, CursorUnavailable> {
    let (path, ambiguous_name) = managed()
        .map(|path| (path, false))
        .or_else(|| resolve(PUBLISHED_EXECUTABLE).map(|path| (path, false)))
        .or_else(|| resolve(AMBIGUOUS_EXECUTABLE).map(|path| (path, true)))
        .ok_or(CursorUnavailable::NotInstalled)?;
    // A version Bridge cannot read is reported as such rather than treated as
    // new enough. The string is a bare date and hash with no name in it, so
    // there is nothing here to parse beyond "the executable answered".
    let version = version_at(&path)
        .map(|version| version.trim().to_owned())
        .filter(|version| !version.is_empty())
        .ok_or_else(|| CursorUnavailable::UnreadableVersion { path: path.clone() })?;
    Ok(CursorExecutable {
        path,
        ambiguous_name,
        version,
    })
}

pub fn locate() -> Result<CursorExecutable, CursorUnavailable> {
    locate_with(
        &|| crate::managed_runtime::managed_entrypoint(HARNESS_ID),
        &binary::resolve,
        &binary::version_at,
    )
}

/// The copy of the vendor's CLI on PATH, under either of the names [`locate`]
/// accepts.
///
/// Exported so surfaces that describe a user's own install — the managed
/// runtimes card, and the uninstall guard behind it — resolve the same file this
/// adapter would launch. Looking only for the published name there reported a
/// working install as absent and offered to download a second copy of it.
pub fn system_executable() -> Option<PathBuf> {
    binary::resolve(PUBLISHED_EXECUTABLE).or_else(|| binary::resolve(AMBIGUOUS_EXECUTABLE))
}

/// The executable a sign-in may be spawned against.
///
/// Narrower than [`locate`] on purpose. A login is an interactive vendor process
/// in a Bridge-owned PTY labelled Cursor, and nothing has asked the ambiguous
/// name who it is at that point — the identity check lives in [`probe`], which a
/// login never reaches. So a managed payload or the vendor's own published name,
/// and otherwise the same remedy an unidentified build already names.
pub fn login_executable() -> Result<PathBuf, CursorUnavailable> {
    login_executable_of(locate()?)
}

fn login_executable_of(executable: CursorExecutable) -> Result<PathBuf, CursorUnavailable> {
    if executable.ambiguous_name {
        return Err(CursorUnavailable::Unidentified {
            version: executable.version,
            reported: None,
        });
    }
    Ok(executable.path)
}

/// A configured key, read from Bridge's own environment.
///
/// Returned rather than stored, and never cached: a key belongs to the launch
/// that uses it. The variables are the vendor's own, so a user who has already
/// exported one for the CLI does not export a second one for Bridge.
fn configured_key() -> Option<String> {
    KEY_VARIABLES.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    })
}

/// Build the launch for one Cursor process.
///
/// The credential boundary is this function. A key travels in the environment
/// of the child and nowhere else: the vendor also accepts `--api-key`, and
/// Bridge deliberately does not use it, because an argument vector is readable
/// by every process on the machine and is recorded by Bridge's own process
/// ledger. The argument list is the subcommand alone.
fn launch_for(executable: &Path, cwd: &Path, key: Option<&str>, timeout: Duration) -> AcpLaunch {
    let mut launch = AcpLaunch::new(executable, cwd)
        .arg(ACP_SUBCOMMAND)
        .handshake_timeout(timeout);
    if let Some(key) = key {
        launch = launch.env(KEY_VARIABLES[0], key);
    }
    launch
}

/// Remove a configured key from text that is about to be reported.
///
/// Failure context is a bounded tail of whatever the agent wrote, and an agent
/// that echoes its own configuration into a diagnostic would otherwise put a
/// credential into an event, a log line, and a session row. Redaction happens
/// at the reporting boundary rather than at the capture boundary because the
/// capture belongs to the shared client, which has no idea what is secret.
fn redact(text: &str, key: Option<&str>) -> String {
    match key.filter(|key| !key.is_empty()) {
        Some(key) => text.replace(key, "[redacted]"),
        None => text.to_owned(),
    }
}

/// Confirm that this build speaks ACP without opening a provider session.
///
/// `session/new` can boot every MCP server the provider has configured. That
/// work belongs to an actual user session, not an availability refresh. Models
/// and modes are therefore learned on the first real launch and cached there.
fn probe(
    executable: &CursorExecutable,
    timeout: Duration,
) -> Result<CursorProfile, CursorUnavailable> {
    let key = configured_key();
    let launch = launch_for(
        &executable.path,
        &std::env::temp_dir(),
        key.as_deref(),
        timeout,
    )
    .ledger_kind("acp.discovery");
    let capabilities = match AcpSession::probe(launch) {
        Ok(capabilities) => capabilities,
        Err(error) => {
            return Err(classify_probe_failure(
                &executable.version,
                &error,
                key.as_deref(),
            ))
        }
    };
    let profile = read_profile(executable, &capabilities, &AcpSessionState::default());
    let profile = profile?;
    // The bare name resolves an unrelated vendor's CLI too, so it only counts
    // once the executable has said who it is. The published name needs no such
    // corroboration: it is the vendor's own, and nothing else claims it.
    if executable.ambiguous_name && !identifies_as_cursor(profile.agent_name.as_deref()) {
        return Err(CursorUnavailable::Unidentified {
            version: executable.version.clone(),
            reported: profile.agent_name,
        });
    }
    Ok(profile)
}

fn identifies_as_cursor(agent_name: Option<&str>) -> bool {
    agent_name.is_some_and(|name| name.to_ascii_lowercase().contains(VENDOR_MARKER))
}

/// Turn a failed handshake into the reason a user sees.
///
/// Pure, and separate from [`probe`], because every one of these arms is a
/// different remedy and each has to be reachable in a test without arranging a
/// process that fails in exactly that way. A build that answered with terminal
/// output is distinguished from one that answered with nothing at all: the
/// shared client keeps non-protocol stdout in a bounded tail precisely so that
/// distinction survives to here.
fn classify_probe_failure(version: &str, error: &AcpError, key: Option<&str>) -> CursorUnavailable {
    match error {
        AcpError::Launch { reason } => CursorUnavailable::ProbeFailed {
            version: version.to_owned(),
            reason: redact(reason, key),
        },
        AcpError::HandshakeTimeout { output, .. } | AcpError::HandshakeFailed { output, .. } => {
            CursorUnavailable::NotProtocol {
                version: version.to_owned(),
                output: output.as_deref().map(|output| redact(output, key)),
            }
        }
        AcpError::AuthenticationRequired { .. } => CursorUnavailable::NeedsSignIn {
            version: version.to_owned(),
        },
        other => CursorUnavailable::ProbeFailed {
            version: version.to_owned(),
            reason: redact(&other.to_string(), key),
        },
    }
}

/// Build a profile from an initialize-only discovery handshake.
///
/// ACP agents report model selectors and modes only after `session/new`; those
/// fields stay empty until [`profile_after_session`] refreshes the cache after
/// a user-owned launch.
fn read_profile(
    executable: &CursorExecutable,
    capabilities: &AcpCapabilities,
    state: &AcpSessionState,
) -> Result<CursorProfile, CursorUnavailable> {
    let models = model_options(&state.config_options);
    let default_model = models
        .iter()
        .find(|model| model.default_for_tier && model.tier == CapabilityTier::Standard)
        .or_else(|| models.iter().find(|model| model.default_for_tier))
        .map(|model| model.id.clone());
    Ok(CursorProfile {
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
        session_opened: false,
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

/// Refresh the parts an agent only reports after opening a real session.
/// Discovery remains `initialize`-only; this profile is cached after the
/// first user-owned launch so later model pickers stay fully populated.
fn profile_after_session(profile: &CursorProfile, session: &AcpSession) -> CursorProfile {
    let mut refreshed = profile.clone();
    refreshed.session_opened = true;
    refreshed.modes = session
        .session_state()
        .modes
        .as_ref()
        .map(|modes| {
            modes
                .available_modes
                .iter()
                .map(|mode| mode.id.0.to_string())
                .collect()
        })
        .unwrap_or_default();
    refreshed.current_mode = session
        .session_state()
        .modes
        .as_ref()
        .map(|modes| modes.current_mode_id.0.to_string());
    refreshed.models = model_options(&session.session_state().config_options);
    refreshed.default_model = refreshed
        .models
        .iter()
        .find(|model| model.default_for_tier && model.tier == CapabilityTier::Standard)
        .or_else(|| refreshed.models.iter().find(|model| model.default_for_tier))
        .map(|model| model.id.clone());
    refreshed
}

/// Which advertised authentication method Bridge would use.
///
/// Read off the advertisement rather than hardcoded, so a build that renames
/// its method is followed rather than second-guessed, and a build that
/// advertises none leaves the harness with no way to sign in from here — which
/// is reported, not papered over. Bridge performs no authentication itself;
/// this is what it would name if it did.
pub fn advertised_auth_method(profile: &CursorProfile) -> Option<&str> {
    profile.auth_methods.first().map(String::as_str)
}

/// Turn the session's model selector into Bridge's model options.
///
/// Identifiers are copied, never built. Cursor's picker exposes two mutually
/// exclusive identifier namespaces and neither matches the spelling its command
/// line takes, so a label-derived id is rejected for every model with an
/// invalid-params error that reads like a lapsed subscription. Labels are the
/// agent's too.
///
/// ACP does not rank models by capability. Keep the catalog deliberately
/// unranked (represented by Bridge's neutral Standard tier) instead of turning
/// vendor display order into a false Fast/Standard/Strong claim.
fn model_options(options: &[SessionConfigOption]) -> Vec<ModelOption> {
    let Some(select) = model_selector(options) else {
        return Vec::new();
    };
    let values = flatten_options(&select.options);
    let current = select.current_value.0.to_string();
    let models: Vec<ModelOption> = values
        .into_iter()
        .map(|(id, label)| ModelOption {
            id,
            label,
            tier: CapabilityTier::Standard,
            available: true,
            compatible: true,
            lifecycle: ModelLifecycle::Unknown,
            source: ModelCatalogSource::RuntimeApi,
            supported_effort_levels: Vec::new(),
            default_for_tier: false,
        })
        .collect();
    model_catalog::normalize(
        ModelCatalogSource::RuntimeApi,
        models.into_iter().map(|model| {
            let provider_default = model.id == current;
            CatalogCandidate {
                id: model.id,
                label: model.label,
                tier: model.tier,
                available: model.available,
                compatible: model.compatible,
                lifecycle: if provider_default {
                    ModelLifecycle::Stable
                } else {
                    model.lifecycle
                },
                supported_effort_levels: model.supported_effort_levels,
                promotion_priority: i64::from(provider_default),
            }
        }),
    )
}

/// The selector that offers models.
///
/// Chosen by the category the agent declared, not by the option's id: the id is
/// the agent's namespace and an agent that names its model selector something
/// else is still declaring what it is. An agent that declares no category at
/// all offers no models here rather than having one guessed for it.
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

/// Flatten grouped and ungrouped selector values into (id, label) pairs,
/// preserving the order the agent advertised them in.
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

/// What Bridge can and cannot see of the context Cursor presents to the model.
///
/// Every class is unavailable, and the reason is the protocol rather than a
/// gap in this adapter: ACP carries a prompt, session updates, and permission
/// round trips, and has no message in which an agent declares the instructions,
/// tool schemas, connectors, skills, or sub-agent definitions it assembled
/// behind them. Recorded as stated absences rather than omitted, because a
/// missing observation and an empty one mean different things to the accounting
/// that reads this.
pub(crate) fn cursor_context_inventory(
    lifecycle_phase: ContextLifecyclePhase,
) -> Result<Vec<AdapterContextInventory>, BridgeError> {
    let observations = || {
        vec![
            ContextSegmentObservation::unavailable(
                ContextSegmentClass::ProviderBaseInstructions,
                match lifecycle_phase {
                    ContextLifecyclePhase::Start => "The agent protocol has no field in which Cursor reports the base instructions it opens a session with",
                    ContextLifecyclePhase::Resume => "The agent protocol has no field in which Cursor reports the base instructions retained for a reloaded session",
                    ContextLifecyclePhase::PerTurn => "A prompt carries content blocks and nothing about the provider instructions Cursor prepends to them",
                },
            ),
            ContextSegmentObservation::unavailable(
                ContextSegmentClass::ToolSchemas,
                "Cursor reports tool calls as they happen and never the schemas it presented to the model",
            ),
            ContextSegmentObservation::unavailable(
                ContextSegmentClass::McpDynamicTools,
                "Bridge names the connectors a session may use and Cursor does not report which of them reached the model",
            ),
            ContextSegmentObservation::unavailable(
                ContextSegmentClass::SkillsPlugins,
                "Cursor advertises slash commands without saying which skills or plugins contribute context behind them",
            ),
            ContextSegmentObservation::unavailable(
                ContextSegmentClass::AgentDefinitions,
                "Cursor runs its own sub-agents and the protocol carries no definition of them",
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

/// One approval the agent is waiting on, as Bridge saw it go past on the event
/// stream.
///
/// Both halves are the agent's: the id is what must be echoed back, and the
/// kind is the protocol's own classification of what that id means. Cursor
/// spells its ids `allow-once` and `reject-once` — deliberately not the
/// protocol's snake_case kind names — which is exactly why the two are recorded
/// separately and why the id is never derived from the kind.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OfferedOption {
    id: String,
    kind: String,
}

/// Choose which offered option answers a Bridge approval decision.
///
/// Selection is by kind, because that is the protocol's shared vocabulary;
/// the return value is the id, because that is the agent's. Fallbacks only
/// ever narrow: Bridge's `acceptForSession` settles for the single-use allow
/// where no standing one was offered, but a one-time `accept` is never
/// widened into `allow_always` — a user's Allow-once click must not become a
/// standing grant that silences every later request. A decision with no
/// matching kind is refused rather than answered with the nearest thing.
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

/// A running Cursor session.
///
/// Every method here is a thin translation onto [`AcpSession`]; nothing about
/// the protocol is re-implemented. The two pieces of state that are genuinely
/// Bridge's are the turn label the supervisor reads and the approvals seen on
/// the way past, which is the only place the offered option ids exist by the
/// time a user answers one.
pub struct CursorRuntime {
    session: Arc<AcpSession>,
    provider_session_id: String,
    current_turn: Arc<Mutex<Option<String>>>,
    approvals: Arc<Mutex<BTreeMap<u64, Vec<OfferedOption>>>>,
    /// `None` once the runtime has stopped or the pump has ended. Dropping
    /// this sender is what lets the reader reach end of file: the supervisor's
    /// reader thread — the thread that removes this runtime and settles the
    /// session — blocks until every sender is gone, and a sender held for the
    /// runtime's whole lifetime would park it forever on a provider that died
    /// on its own. Shared with the pump so provider death clears it too.
    events: Arc<Mutex<Option<Arc<crate::frame_queue::FrameSender>>>>,
    /// The compiled instruction stack, delivered as the preamble of the first
    /// turn. ACP has no system-prompt channel, so the first prompt is the one
    /// place the delegation protocol, memory packet, and restoration context
    /// can actually reach the agent.
    pending_instructions: Mutex<Option<String>>,
    pumping: Arc<AtomicBool>,
    context_inventory: Mutex<Vec<AdapterContextInventory>>,
    stopped: bool,
}

impl CursorRuntime {
    fn closed(&self) -> BridgeError {
        BridgeError::Invalid(format!(
            "The Cursor session is no longer running{}",
            self.session
                .failure_context()
                .map(|context| format!(": {}", redact(&context, configured_key().as_deref())))
                .unwrap_or_default()
        ))
    }
}

impl AdapterRuntime for CursorRuntime {
    fn process_id(&self) -> u32 {
        self.session.process_id().unwrap_or_default()
    }

    fn provider_session_id(&self) -> &str {
        &self.provider_session_id
    }

    fn current_turn(&self) -> Arc<Mutex<Option<String>>> {
        self.current_turn.clone()
    }

    /// The shared client's queue depth and eviction count, on the same seam
    /// every other harness reports through. Without it a Cursor stream
    /// shortened under pressure is exactly the silent hole the queue's own
    /// eviction counter exists to prevent.
    fn event_queue_metrics(&self) -> Option<crate::frame_queue::QueueMetricsSnapshot> {
        Some(self.session.queue_metrics())
    }

    fn context_inventory(&self) -> Vec<AdapterContextInventory> {
        self.context_inventory.lock().unwrap().clone()
    }

    /// A turn is dispatched to its own thread because the protocol's prompt is
    /// a request that does not return until the agent has finished answering,
    /// and this method's caller is a command handler that must.
    ///
    /// The outcome is not returned here: it reaches the session the same way
    /// every other piece of the turn does, as an event. A prompt that fails
    /// outright is published as a runtime failure rather than dropped, because
    /// a turn that neither streams nor ends is indistinguishable from a hang.
    fn send_turn(&self, text: &str) -> Result<(), BridgeError> {
        self.send_turn_with_context(text, crate::adapters::TurnContext::default())
    }

    /// ACP has no system channel: the compiled prompt already reaches the
    /// agent folded into the first user message. Bridge's per-turn context
    /// joins that same preamble, which is the only channel there is.
    fn send_turn_with_context(
        &self,
        text: &str,
        context: crate::adapters::TurnContext<'_>,
    ) -> Result<(), BridgeError> {
        self.send_turn_with_images(text, context, &[])
    }

    fn supports_images(&self) -> bool { self.session.capabilities().prompt_images }

    fn send_turn_with_images(&self, text: &str, context: crate::adapters::TurnContext<'_>, images: &[bridge_protocol::messages::TurnImage]) -> Result<(), BridgeError> {
        if !images.is_empty() && !self.supports_images() {
            return Err(BridgeError::Invalid("This Cursor runtime does not accept image attachments".into()));
        }
        let images = images.to_vec();
        if self.session.is_closed() {
            return Err(self.closed());
        }
        let events = self
            .events
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| self.closed())?;
        // ACP has no turn identifiers, so the turn.started every other harness
        // emits is synthesized here with Bridge's own id. The supervisor's
        // turn.started arm is the only writer of active_turn_id, the prompt
        // binding, and the per-turn delegation budget key — a harness that
        // never emits one leaves every per-turn ceiling unbound.
        let turn_id = format!("turn-{}", uuid::Uuid::new_v4());
        let mut started = NormalizedEvent::new("turn.started");
        started.data = json!({"turnId": turn_id});
        let text = crate::adapters::folded_message(
            self.pending_instructions.lock().unwrap().take(),
            context,
            text,
        );
        let session = self.session.clone();
        thread::Builder::new()
            .name("cursor-turn".into())
            .spawn(move || {
                // Backpressure must not park send_turn while its caller holds
                // the adapter map: the reader may need that map to drain.
                if events.send_durable(encode_event(&started)).is_err() { return; }
                if let Err(error) = session.prompt_with_images(&text, &images) {
                    let reason = redact(&error.to_string(), configured_key().as_deref());
                    let event = crate::acp_events::runtime_failed_event(error.code(), &reason);
                    drop(events.send_durable(encode_event(&event)));
                }
            })
            .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        crate::context_inventory::record_runtime_inventory(
            &self.context_inventory,
            cursor_context_inventory(ContextLifecyclePhase::PerTurn)?,
        );
        Ok(())
    }

    fn interrupt(&self) -> Result<(), BridgeError> {
        self.session
            .cancel()
            .map_err(|error| BridgeError::Invalid(error.to_string()))
    }

    /// Answer a permission with one of the ids the agent offered.
    ///
    /// The offered ids come from the request Bridge already saw; nothing is
    /// reconstructed from the decision word. An agent that offers a decision no
    /// option covers is told nothing rather than told the wrong thing — the
    /// approval stays outstanding and the error names the decision.
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
            BridgeError::Invalid(format!("Cursor approval id {request_id} is not a number"))
        })?;
        let options = self
            .approvals
            .lock()
            .unwrap()
            .get(&request_id)
            .cloned()
            .ok_or_else(|| {
                BridgeError::Invalid("This Cursor approval is no longer outstanding".into())
            })?;
        let option_id = if let Some(exact) = exact_option_id {
            let offered = options
                .iter()
                .find(|option| option.id == exact)
                .ok_or_else(|| {
                    BridgeError::Invalid(format!("Cursor did not offer option {exact:?}"))
                })?;
            let compatible = option_for_decision(decision, std::slice::from_ref(offered));
            compatible.ok_or_else(|| {
                BridgeError::Invalid(format!(
                    "Cursor option {exact:?} does not represent decision {decision:?}"
                ))
            })?
        } else {
            option_for_decision(decision, &options).ok_or_else(|| {
                BridgeError::Invalid(format!(
                    "Cursor did not offer an option for the decision {decision:?}"
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
            .map(|context| redact(&context, configured_key().as_deref()))
    }

    fn stop(&mut self, reason: ShutdownReason) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        self.session.shutdown(reason);
        // The pump keeps running until the shutdown's own terminal event has
        // been delivered; clearing the flag only tells it to stop looking for
        // more once the queue is empty.
        self.pumping.store(false, Ordering::Release);
        // Release the runtime's own sender so the reader can reach end of
        // file: the pump's clone goes when the pump returns, and this one must
        // not outlive the stop that made further events impossible.
        drop(self.events.lock().unwrap().take());
    }
}

impl Drop for CursorRuntime {
    fn drop(&mut self) {
        self.stop(ShutdownReason::AppShutdown);
    }
}

/// The session's event queue, as the newline-delimited stream every adapter
/// hands back.
///
/// Bridge's reader contract is a `BufRead` a supervisor thread blocks on, and
/// the shared client's is a queue a caller drains. This is the join between
/// them: a channel whose receiving end reads as lines, so nothing downstream
/// has to know that this harness produces typed events rather than parsing
/// them.
struct CursorEventReader {
    lines: crate::frame_queue::FrameReceiver,
    pending: Vec<u8>,
    consumed: usize,
}

impl Read for CursorEventReader {
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

impl BufRead for CursorEventReader {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        if self.consumed == self.pending.len() {
            match self.lines.recv() {
                Ok(line) => {
                    self.pending = line.into_bytes();
                    self.pending.push(b'\n');
                    self.consumed = 0;
                }
                // Every sender is gone, which is this stream's end of file.
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

/// One normalized event, as a line.
///
/// A private encoding rather than a serde derive on [`NormalizedEvent`]: the
/// type is Bridge's internal shape and giving it a wire format would invite
/// every other adapter to grow one. Only this module writes these lines and
/// only this module reads them back.
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

fn send_cursor_event(sender: &crate::frame_queue::FrameSender, event: &NormalizedEvent) -> Result<(), crate::frame_queue::Disconnected> {
    // Prose is lossless at this boundary: a slow reader backpressures its
    // producer instead of relying on a later assembled message to repair it.
    // Thinking has an authoritative completion supplied by AcpThoughtRun.
    if event.kind == "reasoning.delta" {
        sender.send_transient(encode_event(event)).map(|_| ())
    } else {
        sender.send_durable(encode_event(event))
    }
}

/// Move the session's events onto the reader until the session is done.
///
/// The offered options of every permission are recorded on the way past. This
/// is the only point at which they exist: the request arrives, becomes an
/// event, and is answered later by a user, by which time nothing else holds the
/// ids the agent actually offered.
fn pump_events(
    session: Arc<AcpSession>,
    events: Arc<crate::frame_queue::FrameSender>,
    runtime_sender: Arc<Mutex<Option<Arc<crate::frame_queue::FrameSender>>>>,
    approvals: Arc<Mutex<BTreeMap<u64, Vec<OfferedOption>>>>,
    pumping: Arc<AtomicBool>,
) {
    'pump: loop {
        let drained = session.drain();
        let idle = drained.is_empty();
        for event in &drained {
            record_approval(&approvals, event);
            if send_cursor_event(&events, event).is_err() {
                break 'pump;
            }
        }
        if idle {
            if session.is_closed() || !pumping.load(Ordering::Acquire) {
                // One last pass: the terminal event is pushed before the
                // connection marks itself closed, so a drain after observing
                // closed cannot miss it.
                for event in session.drain() {
                    record_approval(&approvals, &event);
                    if send_cursor_event(&events, &event).is_err() {
                        break 'pump;
                    }
                }
                break 'pump;
            }
            thread::sleep(EVENT_POLL_INTERVAL);
        }
    }
    // The pump ending means no further events can exist. Releasing the
    // runtime's sender here — not only on an explicit stop — is what lets the
    // reader reach end of file when the provider dies on its own, and it is
    // the reader's thread that removes the runtime and settles the session.
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

/// Start one Cursor session in `cwd`.
fn launch(
    profile: &CursorProfile,
    cwd: &str,
    model: Option<&str>,
    instructions: Option<&str>,
    on_progress: Option<crate::adapters::StartupProgress<'_>>,
) -> Result<(StartedAdapter, CursorProfile), BridgeError> {
    let key = configured_key();
    if let Some(on_progress) = on_progress {
        on_progress(StartupPhase::Spawning);
    }
    let session = AcpSession::connect(launch_for(
        &profile.executable,
        Path::new(cwd),
        key.as_deref(),
        crate::acp_session::DEFAULT_HANDSHAKE_TIMEOUT,
    ))
    .map_err(|error| launch_error(&error, key.as_deref()))?;
    let established_profile = profile_after_session(profile, &session);
    if let Some(on_progress) = on_progress {
        on_progress(StartupPhase::Handshake);
    }
    if let Some(model) = model {
        if let Err(error) = apply_model(&session, model) {
            session.shutdown(ShutdownReason::Failed);
            return Err(error);
        }
    }
    let session = Arc::new(session);
    if let Some(on_progress) = on_progress {
        on_progress(StartupPhase::SessionOpen);
    }
    let (sender, receiver, _) = crate::frame_queue::bounded_frame_queue(crate::frame_queue::QueueBudget::default());
    let sender = Arc::new(sender);
    let approvals = Arc::new(Mutex::new(BTreeMap::new()));
    let pumping = Arc::new(AtomicBool::new(true));
    let runtime_sender = Arc::new(Mutex::new(Some(sender.clone())));
    thread::Builder::new()
        .name("cursor-events".into())
        .spawn({
            let session = session.clone();
            let sender = sender.clone();
            let runtime_sender = runtime_sender.clone();
            let approvals = approvals.clone();
            let pumping = pumping.clone();
            move || pump_events(session, sender, runtime_sender, approvals, pumping)
        })
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let runtime = CursorRuntime {
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
        context_inventory: Mutex::new(cursor_context_inventory(ContextLifecyclePhase::Start)?),
        stopped: false,
    };
    Ok((
        StartedAdapter {
            runtime: Box::new(runtime),
            reader: Box::new(CursorEventReader {
                lines: receiver,
                pending: Vec::new(),
                consumed: 0,
            }),
            // Nothing is emitted ahead of the reader: the pump starts holding the
            // same queue the handshake filled, so anything the agent said before
            // this point arrives on the stream like everything else.
            startup_messages: Vec::new(),
        },
        established_profile,
    ))
}

/// Select a model on a session that has just opened.
///
/// The identifier travels exactly as the agent advertised it, and a model the
/// agent did not advertise never reaches the wire. Both halves matter: Cursor
/// answers an identifier from the wrong namespace with an invalid-params error
/// for every model, which a user reads as their subscription having lapsed
/// rather than as Bridge having invented a name.
fn apply_model(session: &AcpSession, model: &str) -> Result<(), BridgeError> {
    let models = model_options(&session.session_state().config_options);
    let advertised = models
        .iter()
        .find(|option| option.id == model)
        .ok_or_else(|| {
            BridgeError::Invalid(format!(
                "Cursor model {model:?} is not one this build of the agent offers"
            ))
        })?;
    let Some(option_id) = model_selector_id(&session.session_state().config_options) else {
        return Err(BridgeError::Invalid(
            "Cursor did not advertise a model selector for this session".into(),
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

/// The id of the selector [`model_selector`] read the models from.
///
/// The same predicate on purpose: an agent that declares a non-select option
/// under the model category — an "auto-select" toggle, say — must not receive
/// a model identifier addressed to it while the models came from somewhere
/// else, because Cursor answers a value from the wrong namespace with an
/// invalid-params error for every model at once.
fn model_selector_id(options: &[SessionConfigOption]) -> Option<String> {
    options
        .iter()
        .find(|option| {
            matches!(option.category, Some(SessionConfigOptionCategory::Model))
                && matches!(option.kind, SessionConfigKind::Select(_))
        })
        .map(|option| option.id.0.to_string())
}

/// A launch that never got as far as a session, as the error a caller sees.
///
/// An agent that wants a sign-in is reported as wanting one, with its own
/// command named. Everything else keeps the shared client's own wording, minus
/// any configured key that turned up in the output it captured.
fn launch_error(error: &AcpError, key: Option<&str>) -> BridgeError {
    match error {
        AcpError::AuthenticationRequired { .. } => {
            BridgeError::Invalid(format!("Cursor is not signed in; run {SIGN_IN_COMMAND}"))
        }
        other => BridgeError::Invalid(redact(&other.to_string(), key)),
    }
}

/// The registered harness.
///
/// Discovery runs off-thread at construction for the same reason OpenCode's
/// does: it spawns a child and reads a handshake, and construction happens
/// during application setup where nothing may block. Every read of the
/// descriptor after that is a cache lookup.
pub struct CursorAdapter {
    probe: Arc<RwLock<Option<CachedProbe>>>,
    /// Held for the length of one probe. A probe costs a vendor process and a
    /// handshake, and the background pass and a caller that arrived before it
    /// landed both reach for one — without this they race and spawn two.
    probing: Arc<Mutex<()>>,
    /// Fired after any off-thread probe lands — the startup pass and every
    /// refresh — so the host can tell the frontend to re-read availability.
    /// Kept rather than consumed: a probe is not a once-per-process event.
    notify: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl CursorAdapter {
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
            .name("cursor-discover".into())
            .spawn(move || {
                drop(store_probe(&probe, &probing));
                if let Some(notify) = notify {
                    notify();
                }
            });
        adapter
    }

    /// The cached probe, taking one if the background pass has not landed or
    /// the build underneath has changed. Blocking, and therefore never called
    /// from [`crate::adapters::HarnessAdapter::descriptor`].
    fn profile(&self) -> Result<CursorProfile, CursorUnavailable> {
        let executable = locate()?;
        if let Some(cached) = self.probe.read().unwrap().as_ref() {
            if cached.describes(&executable) {
                return cached.outcome.clone();
            }
        }
        store_probe(&self.probe, &self.probing)
    }

    fn cached(&self) -> Option<Result<CursorProfile, CursorUnavailable>> {
        self.probe
            .read()
            .unwrap()
            .as_ref()
            .map(|cached| cached.outcome.clone())
    }
}

#[cfg(test)]
impl CursorAdapter {
    /// An adapter holding one already-taken probe and no discovery thread, so a
    /// descriptor can be read without a machine that has the vendor installed.
    fn with_probe(outcome: Result<CursorProfile, CursorUnavailable>) -> Self {
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

/// Locate, probe, and record. Shared by the background pass and by a caller
/// that arrived before it finished.
///
/// One probe at a time, and whoever waits behind one re-reads the cache first:
/// the answer the holder just wrote is the answer the waiter came for, and
/// spawning a second vendor process to ask the same question again would cost
/// another handshake for a result already on the shelf.
fn store_probe(
    cache: &Arc<RwLock<Option<CachedProbe>>>,
    gate: &Arc<Mutex<()>>,
) -> Result<CursorProfile, CursorUnavailable> {
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

/// Probe again even though the build looks unchanged.
///
/// [`store_probe`]'s shortcut answers from the cache whenever the recorded
/// probe describes the executable in front of it — which is exactly wrong after
/// a sign-in, because logging in changes what a probe answers without changing
/// the binary's path or version. This is the same probe with the shortcut left
/// out; the recorded answer stays on the shelf until the fresh one replaces it,
/// so nothing ever reads an emptied cache mid-refresh.
fn refresh_probe(
    cache: &Arc<RwLock<Option<CachedProbe>>>,
    gate: &Arc<Mutex<()>>,
) -> Result<CursorProfile, CursorUnavailable> {
    let _in_flight = gate.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    probe_and_record(cache, locate())
}

fn probe_and_record(
    cache: &Arc<RwLock<Option<CachedProbe>>>,
    located: Result<CursorExecutable, CursorUnavailable>,
) -> Result<CursorProfile, CursorUnavailable> {
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

impl crate::adapters::HarnessAdapter for CursorAdapter {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    /// Take the probe again, off-thread, and say so when it lands.
    ///
    /// The recorded probe is keyed on the binary's path and version, and a
    /// sign-in changes neither — so without this, a user who signs in through
    /// Bridge's own pane keeps reading "not signed in" until the process
    /// restarts. The caller returns immediately; the fresh answer arrives
    /// through the same notify the startup discovery uses.
    fn refresh_availability(&self) {
        let probe = self.probe.clone();
        let probing = self.probing.clone();
        let notify = self.notify.clone();
        let _ = thread::Builder::new()
            .name("cursor-refresh".into())
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
            // `initialize` proves the binary speaks ACP but deliberately does
            // not authenticate. Only a real user session is proof of login;
            // discovery therefore stays honest rather than showing a false
            // signed-in state.
            auth_state: match (profile, unavailable) {
                (Some(profile), _) if profile.session_opened => AuthState::SignedIn,
                (Some(_), _) => AuthState::Unknown,
                (None, Some(reason)) => reason.auth_state(),
                (None, None) => AuthState::Unknown,
            },
            version: profile.map(|profile| profile.version.clone()),
            capabilities: CAPABILITIES.iter().copied().map(str::to_owned).collect(),
            sandbox_modes: crate::builtin_compatibility::CURSOR_SANDBOXES.to_vec(),
            // Every unavailable state names a reason. Discovery that has not
            // landed yet is distinguished from an absent binary without
            // spawning anything: the descriptor is rebuilt on every state read.
            unavailable_reason: profile.is_none().then(|| match unavailable {
                Some(reason) => reason.reason(),
                None if binary::resolve(PUBLISHED_EXECUTABLE).is_none()
                    && binary::resolve(AMBIGUOUS_EXECUTABLE).is_none() =>
                {
                    CursorUnavailable::NotInstalled.reason()
                }
                None => "Cursor has not finished starting".into(),
            }),
            models: profile
                .map(|profile| profile.models.clone())
                .unwrap_or_default(),
            default_model: profile.and_then(|profile| profile.default_model.clone()),
            model_catalog: ModelCatalogDiagnostics {
                source: ModelCatalogSource::RuntimeApi,
                fetched_at: None,
                expires_at: None,
                stale: false,
                last_error: unavailable.map(|reason| reason.reason()),
            },
        }
    }

    /// Start a session, refusing anything this harness cannot actually hold.
    ///
    /// An adapter that quietly drops an isolation request runs an agent with
    /// more authority than the policy engine granted it, and Bridge has already
    /// logged that the isolation was prepared — so a scope Cursor cannot
    /// enforce is refused at the boundary, the way OpenCode refuses a read-only
    /// worker its transport cannot sandbox. The vendor CLI is spawned by the
    /// protocol crate rather than through `worker_sandbox`, so there is no
    /// seatbelt to put around it here.
    fn start(&self, request: StartRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        if request.read_only_sandbox.is_some()
            || matches!(request.write_mode, Some(WriteMode::ReadOnly))
        {
            return Err(BridgeError::Invalid(
                "Cursor read-only workers are unsupported because its CLI is spawned by the agent \
                 protocol client rather than inside the offline sandbox; refusing to start without \
                 isolation"
                    .into(),
            ));
        }
        if request.briefing.is_some() {
            return Err(BridgeError::Invalid(
                "Cursor cannot run a briefing: it has no certified permission representation, so \
                 an empty tool scope cannot be enforced on it"
                    .into(),
            ));
        }
        let profile = self
            .profile()
            .map_err(|reason| BridgeError::Invalid(reason.reason()))?;
        let (started, established_profile) = launch(
            &profile,
            request.cwd,
            request.model,
            request.instructions,
            request.on_progress,
        )?;
        *self.probe.write().unwrap() = Some(CachedProbe {
            executable: established_profile.executable.clone(),
            version: established_profile.version.clone(),
            outcome: Ok(established_profile),
        });
        if let Some(notify) = &self.notify {
            notify();
        }
        Ok(started)
    }

    /// Unreachable while [`Self::supports_native_resume`] is false — the
    /// registry refuses the call before it gets here — and stated rather than
    /// left to a panic, so the reason a Cursor session comes back through a
    /// checkpoint is legible at the place a reader looks for it.
    fn resume(&self, _request: ResumeRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        Err(BridgeError::Invalid(RESUME_UNAVAILABLE.into()))
    }

    /// Cursor is never resumed natively, and the reason is two independent
    /// facts rather than a policy.
    ///
    /// The agent advertises `session/load` — history replay — and does not
    /// advertise `session/resume`; the two are separate capabilities and replay
    /// is not resumption, so treating one as the other would claim a continuity
    /// nothing offered. And a replay would not be enough on its own: the shared
    /// client scopes every turn to the session its handshake opened, so
    /// reconnecting to an earlier one would leave the conversation visible and
    /// the prompts going somewhere else. Bridge hands the session over at a
    /// checkpoint instead, which is what it can actually do.
    fn supports_native_resume(&self) -> bool {
        false
    }

    /// The events arrive already normalized, written by this module's own pump
    /// and read straight back. Nothing about the protocol is re-derived from
    /// JSON here: [`crate::acp_events`] did that against the protocol crate's
    /// typed enums before the line was ever written.
    fn normalize(&self, value: &Value) -> Vec<NormalizedEvent> {
        decode_event(value).into_iter().collect()
    }
}

/// Why a Cursor session comes back through a checkpoint rather than a resume.
const RESUME_UNAVAILABLE: &str =
    "Cursor advertises history replay and not resumption, so Bridge restores its sessions from \
     a checkpoint rather than resuming them natively";

/// What the harness can do, in Bridge's capability vocabulary.
///
/// The published contract owns this list; the descriptor mirrors it, and a
/// conformance test compares the two. `history` is here because the agent
/// advertises `session/load`, and `interrupt` because a cancel is a real
/// protocol notification the runtime sends. `steering` is not, because a
/// second prompt against a live turn would be a second turn. Images are sent
/// through the shared client only when the session advertises image support.
const CAPABILITIES: &[&str] = crate::builtin_compatibility::CURSOR_CAPABILITIES;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp_events::AcpTurnOutcome;
    use crate::adapters::HarnessAdapter;
    use agent_client_protocol::schema::v1::{SessionConfigSelectGroup, SessionConfigSelectOption};
    use std::os::unix::fs::PermissionsExt;

    /// A shim on the search path that behaves like one build of the vendor CLI.
    ///
    /// The behaviour lives in a checked-in fixture and the shim only chooses
    /// which of it to run, so the tests exercise real executables found by real
    /// path resolution rather than a mock of either.
    struct FakeCli {
        mode: &'static str,
        version: &'static str,
        agent_name: &'static str,
    }

    impl FakeCli {
        const fn speaking_protocol() -> Self {
            Self {
                mode: "protocol",
                version: "2026.07.23-e383d2b",
                agent_name: "Cursor Agent",
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
                .join("../../testing/fixtures/cursor-fake-cli.sh");
            let path = directory.join(name);
            std::fs::write(
                &path,
                format!(
                    "#!/bin/sh\n\
                     BRIDGE_CURSOR_FAKE_MODE='{}' \\\n\
                     BRIDGE_CURSOR_FAKE_VERSION='{}' \\\n\
                     BRIDGE_CURSOR_FAKE_AGENT_NAME='{}' \\\n\
                     exec /bin/sh '{}' \"$@\"\n",
                    self.mode,
                    self.version,
                    self.agent_name,
                    fixture.display()
                ),
            )
            .expect("the fake CLI is written");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("the fake CLI is executable");
            path
        }
    }

    /// Resolution over one directory and nothing else, so a CLI that happens to
    /// be installed on the machine running the tests cannot be found instead.
    fn search_path(directory: &Path) -> impl Fn(&str) -> Option<PathBuf> + '_ {
        move |name| {
            let path = directory.join(name);
            path.is_file().then_some(path)
        }
    }

    fn locate_in(directory: &Path) -> Result<CursorExecutable, CursorUnavailable> {
        locate_with(&|| None, &search_path(directory), &binary::version_at)
    }

    fn locate_managed(
        managed: &Path,
        directory: &Path,
    ) -> Result<CursorExecutable, CursorUnavailable> {
        locate_with(
            &|| Some(managed.to_path_buf()),
            &search_path(directory),
            &binary::version_at,
        )
    }

    fn temp_directory() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temp directory")
    }

    fn select_option(value: &str, name: &str) -> SessionConfigSelectOption {
        SessionConfigSelectOption::new(value.to_owned(), name.to_owned())
    }

    fn model_selector_option(
        current: &str,
        options: SessionConfigSelectOptions,
    ) -> SessionConfigOption {
        let mut option = SessionConfigOption::new(
            "model".to_owned(),
            "Model".to_owned(),
            SessionConfigKind::Select(SessionConfigSelect::new(current.to_owned(), options)),
        );
        option.category = Some(SessionConfigOptionCategory::Model);
        option
    }

    fn offered(id: &str, kind: &str) -> OfferedOption {
        OfferedOption {
            id: id.to_owned(),
            kind: kind.to_owned(),
        }
    }

    #[test]
    fn the_published_name_wins_over_the_one_another_vendor_also_claims() {
        let directory = temp_directory();
        FakeCli::speaking_protocol().install(directory.path(), PUBLISHED_EXECUTABLE);
        FakeCli::speaking_protocol().install(directory.path(), AMBIGUOUS_EXECUTABLE);

        let found = locate_in(directory.path()).expect("both names resolve");
        assert_eq!(found.path, directory.path().join(PUBLISHED_EXECUTABLE));
        assert!(
            !found.ambiguous_name,
            "the published name needs no corroboration"
        );
        assert_eq!(found.version, "2026.07.23-e383d2b");
    }

    #[test]
    fn the_bare_name_is_taken_only_when_the_published_one_is_missing() {
        let directory = temp_directory();
        FakeCli::speaking_protocol().install(directory.path(), AMBIGUOUS_EXECUTABLE);

        let found = locate_in(directory.path()).expect("the bare name resolves");
        assert_eq!(found.path, directory.path().join(AMBIGUOUS_EXECUTABLE));
        assert!(
            found.ambiguous_name,
            "finding the shared name is not on its own evidence of the vendor"
        );
    }

    #[test]
    fn a_managed_payload_outranks_both_names_on_path() {
        let directory = temp_directory();
        FakeCli::speaking_protocol().install(directory.path(), PUBLISHED_EXECUTABLE);
        let managed_directory = temp_directory();
        let managed =
            FakeCli::speaking_protocol().install(managed_directory.path(), PUBLISHED_EXECUTABLE);

        let found = locate_managed(&managed, directory.path()).expect("the payload resolves");
        assert_eq!(
            found.path, managed,
            "an installed payload is the copy Bridge was asked to run"
        );
        assert!(
            !found.ambiguous_name,
            "Bridge extracted the payload from its own pinned recipe"
        );
    }

    #[test]
    fn a_sign_in_is_never_spawned_against_the_shared_name() {
        let directory = temp_directory();
        FakeCli::speaking_protocol().install(directory.path(), AMBIGUOUS_EXECUTABLE);
        let ambiguous = locate_in(directory.path()).expect("the bare name resolves");

        let refused = login_executable_of(ambiguous).unwrap_err();
        assert!(matches!(refused, CursorUnavailable::Unidentified { .. }));
        assert!(refused.reason().contains(PUBLISHED_EXECUTABLE));

        let published = temp_directory();
        FakeCli::speaking_protocol().install(published.path(), PUBLISHED_EXECUTABLE);
        let unambiguous = locate_in(published.path()).expect("the published name resolves");
        assert_eq!(
            login_executable_of(unambiguous).unwrap(),
            published.path().join(PUBLISHED_EXECUTABLE)
        );
    }

    #[test]
    fn a_refresh_takes_a_new_probe_even_when_the_build_is_unchanged() {
        let directory = temp_directory();
        FakeCli::speaking_protocol().install(directory.path(), PUBLISHED_EXECUTABLE);
        let executable = locate_in(directory.path()).expect("the fake resolves");
        // Signing in changes neither the path nor the version, so the recorded
        // probe still describes this exact build — which is the shortcut
        // store_probe takes, and the reason a refresh must not.
        let stale = CachedProbe {
            executable: executable.path.clone(),
            version: executable.version.clone(),
            outcome: Err(CursorUnavailable::NeedsSignIn {
                version: executable.version.clone(),
            }),
        };
        assert!(
            stale.describes(&executable),
            "the stale record is exactly what the shortcut would serve"
        );
        let cache = Arc::new(RwLock::new(Some(stale)));
        let refreshed = probe_and_record(&cache, Ok(executable));
        assert!(
            refreshed.is_ok(),
            "a fresh handshake replaces the recorded sign-out: {refreshed:?}"
        );
        let recorded = cache
            .read()
            .unwrap()
            .as_ref()
            .expect("the cache is never emptied mid-refresh")
            .outcome
            .clone();
        assert!(
            recorded.is_ok(),
            "the replacement is what later reads serve"
        );
    }

    #[test]
    fn an_absent_executable_says_what_was_looked_for() {
        let directory = temp_directory();
        let reason = locate_in(directory.path()).unwrap_err();
        assert_eq!(reason, CursorUnavailable::NotInstalled);
        let sentence = reason.reason();
        assert!(sentence.contains(PUBLISHED_EXECUTABLE), "{sentence}");
        assert!(sentence.contains(AMBIGUOUS_EXECUTABLE), "{sentence}");
    }

    #[test]
    fn a_version_that_cannot_be_read_is_not_treated_as_new_enough() {
        let directory = temp_directory();
        let path = FakeCli::speaking_protocol()
            .version("")
            .install(directory.path(), PUBLISHED_EXECUTABLE);

        let reason = locate_in(directory.path()).unwrap_err();
        assert_eq!(reason, CursorUnavailable::UnreadableVersion { path });
        assert!(reason.reason().contains("did not report a version"));
    }

    #[test]
    fn a_probe_reads_capabilities_without_opening_a_session() {
        let directory = temp_directory();
        FakeCli::speaking_protocol().install(directory.path(), PUBLISHED_EXECUTABLE);
        let executable = locate_in(directory.path()).expect("the fake CLI resolves");

        let profile =
            probe(&executable, PROBE_TIMEOUT).expect("the fake CLI completes a handshake");
        assert_eq!(profile.agent_name.as_deref(), Some("Cursor Agent"));
        assert!(profile.load_session, "history replay was advertised");
        assert!(
            !profile.resume_session,
            "resumption was not advertised and must not be inferred from replay"
        );
        assert!(!profile.additional_directories);
        assert_eq!(profile.auth_methods, ["cursor_login"]);
        assert!(!profile.session_opened);
        assert!(profile.modes.is_empty());
        assert!(profile.current_mode.is_none());
        assert!(profile.models.is_empty());
        assert!(profile.default_model.is_none());
    }

    #[test]
    fn a_build_that_answers_the_subcommand_with_terminal_output_fails_the_probe() {
        let directory = temp_directory();
        FakeCli::speaking_protocol()
            .mode("terminal")
            .install(directory.path(), PUBLISHED_EXECUTABLE);
        let executable = locate_in(directory.path()).expect("the fake CLI resolves");

        let reason = probe(&executable, PROBE_TIMEOUT).unwrap_err();
        let CursorUnavailable::NotProtocol { version, output } = &reason else {
            panic!("a terminal-painting build is a failed probe, saw {reason:?}");
        };
        assert_eq!(version, "2026.07.23-e383d2b");
        let output = output
            .as_deref()
            .expect("what it printed is kept as evidence");
        assert!(
            output.contains("Cursor Agent"),
            "the non-protocol output is the explanation: {output}"
        );
        assert!(reason.reason().contains("terminal output"));
    }

    #[test]
    fn the_bare_name_counts_only_once_the_agent_says_who_it_is() {
        let directory = temp_directory();
        FakeCli::speaking_protocol()
            .agent_name("Other Vendor Agent")
            .install(directory.path(), AMBIGUOUS_EXECUTABLE);
        let ambiguous = locate_in(directory.path()).expect("the bare name resolves");

        let reason = probe(&ambiguous, PROBE_TIMEOUT).unwrap_err();
        assert_eq!(
            reason,
            CursorUnavailable::Unidentified {
                version: "2026.07.23-e383d2b".into(),
                reported: Some("Other Vendor Agent".into()),
            },
            "an unrelated agent under the shared name is not this harness"
        );
        assert!(reason.reason().contains(PUBLISHED_EXECUTABLE));

        // The published name is the vendor's own and nothing else claims it, so
        // the same executable found under it is taken at its word.
        let published = temp_directory();
        FakeCli::speaking_protocol()
            .agent_name("Other Vendor Agent")
            .install(published.path(), PUBLISHED_EXECUTABLE);
        let unambiguous = locate_in(published.path()).expect("the published name resolves");
        assert!(probe(&unambiguous, PROBE_TIMEOUT).is_ok());
    }

    #[test]
    fn a_probe_does_not_open_a_session_to_check_login() {
        let directory = temp_directory();
        FakeCli::speaking_protocol()
            .mode("needs_login")
            .install(directory.path(), PUBLISHED_EXECUTABLE);
        let executable = locate_in(directory.path()).expect("the fake CLI resolves");

        let profile = probe(&executable, PROBE_TIMEOUT).unwrap();
        assert!(!profile.session_opened);
    }

    /// Vendor requests are handled only on actual sessions. Discovery must not
    /// reach one, so this fixture's session-new traffic is never triggered.
    #[test]
    fn discovery_never_reaches_vendor_session_traffic() {
        let directory = temp_directory();
        FakeCli::speaking_protocol()
            .mode("vendor_traffic")
            .install(directory.path(), PUBLISHED_EXECUTABLE);
        let executable = locate_in(directory.path()).expect("the fake CLI resolves");

        let profile = probe(&executable, PROBE_TIMEOUT).expect("initialize succeeds");
        assert!(!profile.session_opened);
        assert!(profile.modes.is_empty());
    }

    #[test]
    fn a_probe_is_reused_for_its_own_build_and_dropped_when_the_build_moves() {
        let directory = temp_directory();
        FakeCli::speaking_protocol().install(directory.path(), PUBLISHED_EXECUTABLE);
        let executable = locate_in(directory.path()).expect("the fake CLI resolves");
        let cached = CachedProbe {
            executable: executable.path.clone(),
            version: executable.version.clone(),
            outcome: probe(&executable, PROBE_TIMEOUT),
        };

        assert!(cached.describes(&executable));
        assert!(
            !cached.describes(&CursorExecutable {
                version: "2026.08.01-aaaaaaa".into(),
                ..executable.clone()
            }),
            "a version that moved is a different build"
        );
        assert!(
            !cached.describes(&CursorExecutable {
                path: PathBuf::from("/somewhere/else/cursor-agent"),
                ..executable
            }),
            "a different path is a different install"
        );
    }

    #[test]
    fn a_configured_key_travels_in_the_environment_and_never_on_the_command_line() {
        let launch = launch_for(
            Path::new("/opt/cursor-agent"),
            Path::new("/workspace"),
            Some("sk-cursor-secret"),
            PROBE_TIMEOUT,
        );
        assert_eq!(
            launch.args,
            [ACP_SUBCOMMAND],
            "the subcommand is the whole argument vector"
        );
        assert!(
            !launch
                .args
                .iter()
                .any(|arg| arg.contains("sk-cursor-secret")),
            "an argument vector is readable by every process on the machine"
        );
        assert_eq!(
            launch.env.get(KEY_VARIABLES[0]).map(String::as_str),
            Some("sk-cursor-secret")
        );
        assert_eq!(
            launch_for(
                Path::new("/opt/cursor-agent"),
                Path::new("/workspace"),
                None,
                PROBE_TIMEOUT
            )
            .env
            .len(),
            0,
            "no key configured means nothing added to the child environment"
        );
    }

    #[test]
    fn a_configured_key_never_reaches_anything_that_gets_reported() {
        let key = Some("sk-cursor-secret");
        let leaked = "the agent said: token sk-cursor-secret was rejected";
        let redacted = redact(leaked, key);
        assert!(!redacted.contains("sk-cursor-secret"), "{redacted}");
        assert!(redacted.contains("[redacted]"), "{redacted}");
        assert_eq!(
            redact(leaked, None),
            leaked,
            "nothing configured, nothing to hide"
        );

        let reason = classify_probe_failure(
            "2026.07.23-e383d2b",
            &AcpError::HandshakeFailed {
                reason: "closed".into(),
                output: Some(leaked.into()),
            },
            key,
        );
        assert!(!format!("{reason:?}").contains("sk-cursor-secret"));
        assert!(!reason.reason().contains("sk-cursor-secret"));
    }

    #[test]
    fn every_probe_failure_names_its_own_remedy() {
        let version = "2026.07.23-e383d2b";
        let cases = [
            (
                AcpError::Launch {
                    reason: "permission denied".into(),
                },
                CursorUnavailable::ProbeFailed {
                    version: version.into(),
                    reason: "permission denied".into(),
                },
            ),
            (
                AcpError::HandshakeTimeout {
                    millis: 12_000,
                    output: Some("[?1049h".into()),
                },
                CursorUnavailable::NotProtocol {
                    version: version.into(),
                    output: Some("[?1049h".into()),
                },
            ),
            (
                AcpError::AuthenticationRequired {
                    reason: "Authentication required".into(),
                },
                CursorUnavailable::NeedsSignIn {
                    version: version.into(),
                },
            ),
            (
                AcpError::ProtocolVersion {
                    requested: 1,
                    offered: 2,
                },
                CursorUnavailable::ProbeFailed {
                    version: version.into(),
                    reason: AcpError::ProtocolVersion {
                        requested: 1,
                        offered: 2,
                    }
                    .to_string(),
                },
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(
                classify_probe_failure(version, &error, None),
                expected,
                "{error:?}"
            );
        }
        // Every remedy reads differently, or the reason is not one.
        let sentences: Vec<String> = [
            CursorUnavailable::NotInstalled,
            CursorUnavailable::UnreadableVersion {
                path: PathBuf::from("/opt/cursor-agent"),
            },
            CursorUnavailable::NotProtocol {
                version: version.into(),
                output: None,
            },
            CursorUnavailable::Unidentified {
                version: version.into(),
                reported: None,
            },
            CursorUnavailable::NeedsSignIn {
                version: version.into(),
            },
            CursorUnavailable::ProbeFailed {
                version: version.into(),
                reason: "broke".into(),
            },
        ]
        .iter()
        .map(CursorUnavailable::reason)
        .collect();
        let unique: std::collections::BTreeSet<&String> = sentences.iter().collect();
        assert_eq!(unique.len(), sentences.len(), "{sentences:?}");
    }

    #[test]
    fn model_identifiers_are_echoed_and_never_rebuilt_from_a_label() {
        // The two picker modes use disjoint namespaces and neither matches the
        // command-line spelling, so an identifier derived from anything but the
        // wire is rejected for every model at once.
        let options = model_options(&[model_selector_option(
            "composer-1",
            vec![
                select_option("cheetah", "Cheetah"),
                select_option("composer-1", "Composer 1"),
                select_option("claude-4.5-sonnet", "Claude 4.5 Sonnet"),
            ]
            .into(),
        )]);
        assert_eq!(
            options
                .iter()
                .map(|model| (model.id.as_str(), model.label.as_str()))
                .collect::<Vec<_>>(),
            [
                ("cheetah", "Cheetah"),
                ("claude-4.5-sonnet", "Claude 4.5 Sonnet"),
                ("composer-1", "Composer 1"),
            ]
        );
    }

    #[test]
    fn vendor_order_never_becomes_a_capability_ranking() {
        let options = model_options(&[model_selector_option(
            "b",
            vec![
                select_option("a", "A"),
                select_option("b", "B"),
                select_option("c", "C"),
            ]
            .into(),
        )]);
        assert!(options
            .iter()
            .all(|model| model.tier == CapabilityTier::Standard));
        let selected = options
            .iter()
            .find(|model| model.default_for_tier)
            .expect("a catalog default");
        assert_eq!(selected.id, "b");
        assert_eq!(
            options
                .iter()
                .filter(|model| model.default_for_tier)
                .count(),
            1
        );

        // A single advertised model is a standard model, not a third of one.
        let single = model_options(&[model_selector_option(
            "only",
            vec![select_option("only", "Only")].into(),
        )]);
        assert_eq!(single[0].tier, CapabilityTier::Standard);
        assert!(single[0].default_for_tier);
    }

    #[test]
    fn a_selector_that_is_not_the_model_selector_offers_no_models() {
        // Modes and thought levels arrive through the same mechanism. Reading
        // any selector as a model list would offer "plan" as something to run.
        let mut mode_selector = SessionConfigOption::new(
            "mode".to_owned(),
            "Mode".to_owned(),
            SessionConfigKind::Select(SessionConfigSelect::new(
                "plan".to_owned(),
                SessionConfigSelectOptions::Ungrouped(vec![
                    select_option("agent", "Agent"),
                    select_option("plan", "Plan"),
                ]),
            )),
        );
        mode_selector.category = Some(SessionConfigOptionCategory::Mode);
        assert!(model_options(&[mode_selector]).is_empty());

        let mut thought = SessionConfigOption::new(
            "model".to_owned(),
            "Thinking".to_owned(),
            SessionConfigKind::Select(SessionConfigSelect::new(
                "high".to_owned(),
                SessionConfigSelectOptions::Ungrouped(vec![select_option("high", "High")]),
            )),
        );
        thought.category = Some(SessionConfigOptionCategory::ThoughtLevel);
        assert!(
            model_options(&[thought]).is_empty(),
            "the declared category decides, not the option's id"
        );

        let uncategorized = SessionConfigOption::new(
            "model".to_owned(),
            "Model".to_owned(),
            SessionConfigKind::Select(SessionConfigSelect::new(
                "one".to_owned(),
                SessionConfigSelectOptions::Ungrouped(vec![select_option("one", "One")]),
            )),
        );
        assert!(
            model_options(&[uncategorized]).is_empty(),
            "an undeclared category is not a model selector by inference"
        );
    }

    #[test]
    fn grouped_selector_values_keep_their_own_identifiers() {
        let options = model_options(&[model_selector_option(
            "anthropic/one",
            vec![
                SessionConfigSelectGroup::new(
                    "anthropic".to_owned(),
                    "Anthropic".to_owned(),
                    vec![select_option("anthropic/one", "One")],
                ),
                SessionConfigSelectGroup::new(
                    "openai".to_owned(),
                    "OpenAI".to_owned(),
                    vec![select_option("openai/two", "Two")],
                ),
            ]
            .into(),
        )]);
        assert_eq!(
            options
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            ["anthropic/one", "openai/two"],
            "the group name is a label, never part of the identifier"
        );
        assert_eq!(options[0].label, "Anthropic \u{b7} One");
    }

    #[test]
    fn an_approval_is_answered_with_the_identifier_the_vendor_offered() {
        // Cursor spells its option ids with hyphens, deliberately not matching
        // the protocol's snake_case kinds. Selection reads the kind; the answer
        // is the id, byte for byte.
        let offered_options = [
            offered("allow-once", "allow_once"),
            offered("allow-always", "allow_always"),
            offered("reject-once", "reject_once"),
        ];
        assert_eq!(
            option_for_decision("accept", &offered_options),
            Some("allow-once")
        );
        assert_eq!(
            option_for_decision("acceptForSession", &offered_options),
            Some("allow-always")
        );
        assert_eq!(
            option_for_decision("decline", &offered_options),
            Some("reject-once")
        );
        assert_eq!(
            option_for_decision("cancel", &offered_options),
            Some("reject-once")
        );
    }

    #[test]
    fn a_decision_no_offered_option_covers_is_refused_rather_than_approximated() {
        let only_rejection = [offered("reject-once", "reject_once")];
        assert_eq!(option_for_decision("accept", &only_rejection), None);

        // A standing allow the agent did not offer falls back to the single-use
        // one; nothing widens a decision the other way.
        let single_use = [offered("allow-once", "allow_once")];
        assert_eq!(
            option_for_decision("acceptForSession", &single_use),
            Some("allow-once")
        );
        assert_eq!(option_for_decision("shipit", &single_use), None);

        // The widening direction stays closed: an agent that offers only a
        // standing allow gets no answer to a one-time Accept, because turning
        // one approval into a session-wide grant is a decision the user did
        // not make.
        let standing_only = [
            offered("allow-always", "allow_always"),
            offered("reject-once", "reject_once"),
        ];
        assert_eq!(option_for_decision("accept", &standing_only), None);
        assert_eq!(
            option_for_decision("acceptForSession", &standing_only),
            Some("allow-always")
        );

        // An option whose kind this build has no name for is never selected by
        // its id looking familiar.
        let unknown_kind = [offered("allow-once", "some_future_kind")];
        assert_eq!(option_for_decision("accept", &unknown_kind), None);
    }

    #[test]
    fn a_permission_is_recorded_as_it_passes_and_retired_when_it_settles() {
        let approvals: Arc<Mutex<BTreeMap<u64, Vec<OfferedOption>>>> = Arc::default();
        let mut requested = NormalizedEvent::new("permission.requested");
        requested.data = json!({
            "requestId": 7,
            "options": [
                {"id": "allow-once", "name": "Allow once", "kind": "allow_once"},
                {"id": "reject-once", "name": "Reject", "kind": "reject_once"},
            ],
        });
        record_approval(&approvals, &requested);
        assert_eq!(
            approvals.lock().unwrap().get(&7).cloned(),
            Some(vec![
                offered("allow-once", "allow_once"),
                offered("reject-once", "reject_once"),
            ])
        );

        let mut settled = NormalizedEvent::new("approval.settled");
        settled.data = json!({"requestId": 7, "outcome": "selected"});
        record_approval(&approvals, &settled);
        assert!(approvals.lock().unwrap().is_empty());
    }

    #[test]
    fn an_event_survives_the_round_trip_the_reader_puts_it_through() {
        let mut event = NormalizedEvent::new("tool.progress");
        event.item_id = Some("call-1".into());
        event.status = Some("in_progress".into());
        event.title = Some("Read file".into());
        event.text = Some("src/lib.rs".into());
        event.data = json!({"toolCall": {"toolCallId": "call-1"}});

        let line = encode_event(&event);
        let parsed: Value = serde_json::from_str(&line).expect("the line is one JSON object");
        assert_eq!(decode_event(&parsed), Some(event));
        assert_eq!(
            decode_event(&json!({"data": {}})),
            None,
            "a line with no kind is not an event"
        );
    }

    #[test]
    fn stalled_cursor_reader_preserves_every_acp_message_chunk() {
        use std::sync::mpsc;
        let chunk = |index: usize| crate::acp_events::session_update_event(
            &serde_json::from_value(json!({
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": format!("chunk-{index};") }
            })).unwrap(),
        );
        let (sender, receiver, metrics) = crate::frame_queue::bounded_frame_queue(
            crate::frame_queue::QueueBudget { max_items: 4, max_bytes: 4096 },
        );
        for index in 0..4 { send_cursor_event(&sender, &chunk(index)).unwrap(); }
        let (done_tx, done_rx) = mpsc::channel();
        let producer = thread::spawn(move || {
            for index in 4..1000 { send_cursor_event(&sender, &chunk(index)).unwrap(); }
            done_tx.send(()).unwrap();
        });
        // A stalled consumer must backpressure prose, never evict it. No
        // fabricated message.completed can conceal a lost chunk in this test.
        assert!(matches!(done_rx.recv_timeout(Duration::from_millis(100)), Err(mpsc::RecvTimeoutError::Timeout)));
        assert!(metrics.snapshot().bytes <= 4096);
        let mut received = Vec::new();
        while let Ok(line) = receiver.recv() {
            let event = decode_event(&serde_json::from_str(&line).unwrap()).unwrap();
            assert_eq!(event.kind, "message.delta");
            received.push(event.text.unwrap());
        }
        producer.join().unwrap();
        assert_eq!(received, (0..1000).map(|index| format!("chunk-{index};")).collect::<Vec<_>>());
        assert_eq!(metrics.snapshot().dropped_transient, 0);
    }

    #[test]
    fn stalled_cursor_reader_sheds_only_recoverable_thinking() {
        let (sender, receiver, metrics) = crate::frame_queue::bounded_frame_queue(
            crate::frame_queue::QueueBudget { max_items: 4, max_bytes: 2048 },
        );
        let mut run = crate::acp_events::AcpThoughtRun::default();
        for _ in 0..100 {
            let mut delta = crate::acp_events::session_update_event(
                &serde_json::from_value(json!({
                    "sessionUpdate": "agent_thought_chunk",
                    "content": { "type": "text", "text": "thinking;" }
                })).unwrap(),
            );
            assert!(run.absorb(&mut delta).is_none());
            send_cursor_event(&sender, &delta).unwrap();
        }
        let terminal = run.close().unwrap();
        send_cursor_event(&sender, &terminal).unwrap();
        drop(sender);
        let mut last = None;
        while let Ok(line) = receiver.recv() { last = decode_event(&serde_json::from_str(&line).unwrap()); }
        assert_eq!(last, Some(terminal));
        assert!(metrics.snapshot().dropped_transient > 0);
    }

    #[test]
    fn the_reader_yields_one_line_per_event_and_then_end_of_file() {
        let (sender, receiver, _) = crate::frame_queue::bounded_frame_queue(crate::frame_queue::QueueBudget::default());
        let sender = Arc::new(sender);
        let mut reader = CursorEventReader {
            lines: receiver,
            pending: Vec::new(),
            consumed: 0,
        };
        sender
            .send_durable(encode_event(&NormalizedEvent::new("turn.started")))
            .unwrap();
        sender
            .send_durable(encode_event(&NormalizedEvent::new("turn.completed")))
            .unwrap();
        drop(sender);

        let mut kinds = Vec::new();
        loop {
            let mut line = String::new();
            if reader
                .read_line(&mut line)
                .expect("the reader never errors")
                == 0
            {
                break;
            }
            let value: Value = serde_json::from_str(line.trim()).expect("one event per line");
            kinds.push(decode_event(&value).expect("a decodable event").kind);
        }
        assert_eq!(kinds, ["turn.started", "turn.completed"]);
    }

    #[test]
    fn the_advertised_authentication_method_is_read_rather_than_assumed() {
        let directory = temp_directory();
        FakeCli::speaking_protocol().install(directory.path(), PUBLISHED_EXECUTABLE);
        let executable = locate_in(directory.path()).expect("the fake CLI resolves");
        let profile = probe(&executable, PROBE_TIMEOUT).expect("a handshake");
        assert_eq!(advertised_auth_method(&profile), Some("cursor_login"));

        let unadvertised = CursorProfile {
            auth_methods: Vec::new(),
            ..profile
        };
        assert_eq!(
            advertised_auth_method(&unadvertised),
            None,
            "a method the agent did not advertise is unavailable, not assumed"
        );
    }

    /// A vendor notification Bridge has no handler for is reported, not
    /// dropped: a dropped frame is indistinguishable from one that never
    /// arrived. The fixture sends it during the handshake, so it has been
    /// through the connection by the time the session is open.
    #[test]
    fn a_vendor_notification_is_recorded_as_an_unknown_event() {
        let directory = temp_directory();
        let executable = FakeCli::speaking_protocol()
            .mode("vendor_traffic")
            .install(directory.path(), PUBLISHED_EXECUTABLE);
        let session = AcpSession::connect(launch_for(
            &executable,
            &std::env::temp_dir(),
            None,
            PROBE_TIMEOUT,
        ))
        .expect("the fake CLI completes a handshake");

        let events = session.drain();
        session.shutdown(ShutdownReason::Completed);
        let unknown = events
            .iter()
            .find(|event| event.kind == "provider.unknown")
            .unwrap_or_else(|| panic!("a vendor notification is reported, saw {events:?}"));
        assert_eq!(
            unknown
                .data
                .pointer("/frame/method")
                .and_then(Value::as_str),
            Some("cursor/update_todos")
        );
    }

    fn probed_in(directory: &Path) -> CursorProfile {
        FakeCli::speaking_protocol().install(directory, PUBLISHED_EXECUTABLE);
        let executable = locate_in(directory).expect("the fake CLI resolves");
        probe(&executable, PROBE_TIMEOUT).expect("a handshake")
    }

    fn probed_profile() -> CursorProfile {
        probed_in(temp_directory().path())
    }

    /// Read lines off a started session until one of `kind` arrives.
    ///
    /// The reader blocks on the next event rather than polling, so this waits
    /// on delivery without assuming anything about how long it takes; an agent
    /// that never produces the event ends the stream instead, and the assertion
    /// says what did arrive.
    fn read_until(reader: &mut dyn BufRead, kind: &str) -> Vec<NormalizedEvent> {
        let mut seen = Vec::new();
        loop {
            let mut line = String::new();
            if reader
                .read_line(&mut line)
                .expect("the reader never errors")
                == 0
            {
                panic!("the stream ended before a {kind} event; saw {seen:?}");
            }
            let value: Value = serde_json::from_str(line.trim()).expect("one event per line");
            let event = decode_event(&value).expect("a decodable event");
            let matched = event.kind == kind;
            seen.push(event);
            if matched {
                return seen;
            }
        }
    }

    #[test]
    fn history_replay_is_reported_as_a_checkpoint_handoff_rather_than_as_resume() {
        let profile = probed_profile();
        assert!(profile.load_session && !profile.resume_session);
        let adapter = CursorAdapter::with_probe(Ok(profile.clone()));
        assert!(
            !adapter.supports_native_resume(),
            "replaying history is not resuming a conversation"
        );
        assert!(
            adapter
                .descriptor()
                .capabilities
                .contains(&"history".into()),
            "the replay it does have is still advertised"
        );
        drop(profile);
    }

    #[test]
    fn an_available_descriptor_carries_what_the_session_reported() {
        let profile = probed_profile();
        let descriptor = CursorAdapter::with_probe(Ok(profile.clone())).descriptor();
        assert_eq!(descriptor.id, HARNESS_ID);
        assert_eq!(descriptor.label, HARNESS_LABEL);
        assert!(descriptor.available);
        assert!(descriptor.unavailable_reason.is_none());
        assert_eq!(descriptor.auth_state, AuthState::Unknown);
        assert_eq!(
            descriptor.version.as_deref(),
            Some(profile.version.as_str())
        );
        assert_eq!(descriptor.models, profile.models);
        assert_eq!(descriptor.default_model, profile.default_model);
        assert!(
            !descriptor.capabilities.contains(&"steering".into()),
            "a second prompt against a live turn would be a second turn"
        );
    }

    #[test]
    fn an_unavailable_harness_is_never_selectable_and_always_says_why() {
        for reason in [
            CursorUnavailable::NotInstalled,
            CursorUnavailable::NotProtocol {
                version: "2026.01.01-aaaaaaa".into(),
                output: None,
            },
            CursorUnavailable::NeedsSignIn {
                version: "2026.01.01-aaaaaaa".into(),
            },
        ] {
            let descriptor = CursorAdapter::with_probe(Err(reason.clone())).descriptor();
            assert!(!descriptor.available, "{reason:?}");
            assert_eq!(descriptor.unavailable_reason, Some(reason.reason()));
            assert!(descriptor.models.is_empty(), "{reason:?}");
            assert!(descriptor.default_model.is_none(), "{reason:?}");
            assert!(descriptor.version.is_none(), "{reason:?}");
            assert_eq!(descriptor.auth_state, reason.auth_state());
        }
    }

    #[test]
    fn a_scope_this_harness_cannot_enforce_is_refused_rather_than_run_without_it() {
        // The policy engine admits a read-only worker and Bridge records that
        // isolation was prepared. Starting the vendor CLI outside the sandbox
        // anyway would give the agent authority nobody granted it, so the
        // refusal happens at the boundary — the way OpenCode refuses one.
        let adapter = CursorAdapter::with_probe(Ok(probed_profile()));
        let request = |write_mode, briefing| StartRequest {
            cwd: "/workspace",
            model: None,
            effort: None,
            instructions: None,
            write_mode,
            read_only_sandbox: None,
            briefing,
            on_progress: None,
        };
        let Err(error) = adapter.start(request(Some(WriteMode::ReadOnly), None)) else {
            panic!("a read-only worker must not run unsandboxed");
        };
        assert!(
            error
                .to_string()
                .contains("refusing to start without isolation"),
            "{error}"
        );

        let policy =
            crate::briefing_policy::BriefingRuntimePolicy::compile_scoped(Vec::new(), limits())
                .expect("an empty scope compiles");
        let Err(error) = adapter.start(request(None, Some(&policy))) else {
            panic!("a briefing needs an authority cursor does not have");
        };
        assert!(
            error.to_string().contains("cannot run a briefing"),
            "{error}"
        );
    }

    fn limits() -> bridge_protocol::messages::WorkBriefLimits {
        bridge_protocol::messages::WorkBriefLimits {
            max_wall_seconds: 60,
            max_turns: 1,
            max_tool_calls: 1,
            max_output_tokens: None,
            cost_ceiling_microusd: None,
        }
    }

    #[test]
    fn a_started_session_selects_its_model_by_the_identifier_it_was_given() {
        let directory = temp_directory();
        let profile = probed_in(directory.path());
        let workspace = temp_directory();
        let (started, established_profile) = launch(
            &profile,
            workspace.path().to_str().expect("a utf-8 workspace path"),
            Some("claude-4.5-sonnet"),
            None,
            None,
        )
        .expect("the fake CLI starts a session");
        assert!(established_profile.session_opened);
        assert!(established_profile
            .models
            .iter()
            .any(|model| model.id == "claude-4.5-sonnet"));
        let mut reader = started.reader;

        let echoed = read_until(reader.as_mut(), "provider.unknown")
            .into_iter()
            .find(|event| {
                event.data.pointer("/frame/method").and_then(Value::as_str)
                    == Some("cursor/config_echo")
            })
            .expect("the agent echoes back what it was sent");
        assert_eq!(
            echoed
                .data
                .pointer("/frame/params/value")
                .and_then(Value::as_str),
            Some("claude-4.5-sonnet"),
            "the identifier goes out exactly as the session advertised it"
        );

        started.runtime.send_turn("hello").expect("a turn is sent");
        let completed = read_until(reader.as_mut(), "turn.completed");
        assert_eq!(
            completed
                .last()
                .and_then(|event| event.status.clone())
                .as_deref(),
            Some(AcpTurnOutcome::EndTurn.as_str())
        );

        // Dropping the runtime stops the session and releases the last sender,
        // which is what ends the stream: a reader that outlived its runtime
        // would otherwise block on an event nothing can produce.
        drop(started.runtime);
        let mut trailing = String::new();
        while reader
            .read_line(&mut trailing)
            .expect("the reader never errors")
            != 0
        {
            trailing.clear();
        }
    }

    #[test]
    fn a_model_the_session_never_advertised_never_reaches_the_wire() {
        let directory = temp_directory();
        let profile = probed_in(directory.path());
        let workspace = temp_directory();
        let Err(error) = launch(
            &profile,
            workspace.path().to_str().expect("a utf-8 workspace path"),
            Some("cursor-fast"),
            None,
            None,
        ) else {
            panic!("a model the session never advertised must not start one");
        };
        assert!(
            error.to_string().contains("cursor-fast"),
            "the refused identifier is named: {error}"
        );
    }

    #[test]
    fn a_session_is_never_resumed_natively_and_says_so_when_asked() {
        let adapter = CursorAdapter::with_probe(Ok(probed_profile()));
        assert!(!adapter.supports_native_resume());
        let Err(error) = adapter.resume(ResumeRequest {
            provider_session_id: "cursor-session",
            fork: false,
            cwd: "/workspace",
            model: None,
            effort: None,
            instructions: None,
            write_mode: None,
            read_only_sandbox: None,
            briefing: None,
            on_progress: None,
        }) else {
            panic!("a harness that does not resume must not pretend to");
        };
        assert_eq!(error.to_string(), RESUME_UNAVAILABLE);
    }

    #[test]
    fn a_descriptor_never_carries_a_configured_key() {
        // Every string on the descriptor comes from a path, a version, an
        // identifier the agent advertised, or a reason built from those.
        let profile = probed_profile();
        for descriptor in [
            CursorAdapter::with_probe(Ok(profile)).descriptor(),
            CursorAdapter::with_probe(Err(CursorUnavailable::NeedsSignIn {
                version: "2026.01.01-aaaaaaa".into(),
            }))
            .descriptor(),
        ] {
            let rendered = serde_json::to_string(&descriptor).expect("a serializable descriptor");
            for variable in KEY_VARIABLES {
                assert!(!rendered.contains(variable), "{rendered}");
            }
            for secret_shaped in ["apiKey", "api_key", "token", "Bearer"] {
                assert!(!rendered.contains(secret_shaped), "{rendered}");
            }
        }
    }
}

//! Which implementation serves a public agent, and which one served a session.
//!
//! [`crate::adapters::AdapterRegistry`] is a map of one adapter per id, which
//! makes the agent and its implementation the same thing. That holds exactly as
//! long as every agent has one implementation. A marketplace agent may be
//! reached through an official SDK today and an ACP server next quarter, and a
//! session started under the first must not silently resume under the second.
//!
//! So this module holds the *candidates* — many per agent, ordered by the
//! backend policy — and the registry still holds the executors. Resolution
//! picks a candidate and yields a [`BackendBinding`]; the binding names the
//! adapter the registry then runs. Nothing here launches a process.

use crate::BridgeError;
use bridge_protocol::messages::{AgentId, BackendId, BackendVersion, InstallationId};
use rusqlite::OptionalExtension;
use std::collections::BTreeMap;

/// The strongest official machine interface a backend speaks, in the order the
/// marketplace prefers them.
///
/// The order is the product decision, not an implementation detail: an official
/// SDK is a supported contract, a structured server is a supported protocol, ACP
/// is a shared standard, and a JSON CLI is a stable surface with no session
/// semantics of its own. Terminal-output parsing is deliberately absent — it is
/// a reviewed last resort, and admitting it here as an ordinary tier would make
/// it an ordinary choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BackendKind {
    /// An official vendor SDK, driven in-process or through a Bridge sidecar.
    SdkSidecar,
    /// An official structured application server or API — JSON-RPC over stdio,
    /// or an authenticated loopback HTTP server.
    StructuredServer,
    /// An official Agent Client Protocol server.
    Acp,
    /// An official headless CLI with structured JSON output.
    StructuredCli,
}

impl BackendKind {
    /// Rank in the backend policy. Lower is stronger.
    pub const fn preference(self) -> u8 {
        match self {
            Self::SdkSidecar => 0,
            Self::StructuredServer => 1,
            Self::Acp => 2,
            Self::StructuredCli => 3,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SdkSidecar => "sdk_sidecar",
            Self::StructuredServer => "structured_server",
            Self::Acp => "acp",
            Self::StructuredCli => "structured_cli",
        }
    }
}

/// One way to reach one agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendCandidate {
    pub backend: BackendId,
    /// The [`crate::adapters::AdapterRegistry`] key that executes this backend.
    /// Resolution selects; the registry still runs.
    pub adapter_id: String,
    pub kind: BackendKind,
}

/// What the installed copy of a backend reports about itself: the version, and
/// the managed payload it came from when Bridge owns one.
///
/// Supplied by the caller rather than looked up here. Answering it means
/// digesting a payload tree, and #185 established that the tree is read once per
/// question — a resolver that fetched it again would read it twice.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BackendBacking {
    /// `None` when the backing runtime does not report one. A managed payload
    /// always has a receipt version; a copy on PATH often has nothing until it
    /// is probed, and inventing a version for it would be worse than admitting
    /// there isn't one.
    pub version: Option<BackendVersion>,
    /// `Some` only when Bridge owns the payload. An external runtime has no
    /// receipt, because Bridge installed nothing and may remove nothing.
    pub installation: Option<InstallationId>,
}

/// What actually served a session: the agent a user picked, the implementation
/// that ran, and the exact copy of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendBinding {
    pub agent: AgentId,
    pub backend: BackendId,
    pub version: Option<BackendVersion>,
    pub installation: Option<InstallationId>,
}

impl BackendBinding {
    /// Whether two bindings name the same implementation, ignoring version.
    /// The distinction the continuation rules are built on: a different backend
    /// is a different session, a different version is the same one moving.
    pub fn same_backend(&self, other: &BackendBinding) -> bool {
        self.agent == other.agent && self.backend == other.backend
    }
}

/// Why a backend could not be resolved. One code per condition a caller must be
/// able to act on differently, so nothing has to match a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendError {
    /// The resolver has no candidates for this agent at all.
    UnknownAgent { agent: String },
    /// A session names a backend this build does not have. Its history stays
    /// readable; only resuming it fails.
    BackendUnavailable {
        agent: String,
        backend: String,
        version: Option<String>,
    },
    /// A candidate registration collided with one already present.
    DuplicateBackend { agent: String, backend: String },
    /// The session's recorded binding cannot be interpreted by this build. Its
    /// history still reads; only running it again is refused.
    BindingUnreadable { raw: String, reason: String },
}

impl BackendError {
    /// The stable wire code. Exhaustive on purpose: a new condition must come
    /// past this match and be given its own code rather than reuse one.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::UnknownAgent { .. } => "unknown_agent",
            Self::BackendUnavailable { .. } => "backend_unavailable",
            Self::DuplicateBackend { .. } => "duplicate_backend",
            Self::BindingUnreadable { .. } => "binding_unreadable",
        }
    }
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownAgent { agent } => {
                write!(formatter, "no backend is registered for {agent}")
            }
            Self::BackendUnavailable {
                agent,
                backend,
                version,
            } => match version {
                Some(version) => write!(
                    formatter,
                    "{agent} was served by {backend} {version}, which this build does not have"
                ),
                None => write!(
                    formatter,
                    "{agent} was served by {backend}, which this build does not have"
                ),
            },
            Self::DuplicateBackend { agent, backend } => {
                write!(formatter, "{agent} already has a backend named {backend}")
            }
            Self::BindingUnreadable { raw, reason } => write!(
                formatter,
                "this session records the backend {raw:?}, which this build cannot \
                 interpret: {reason}"
            ),
        }
    }
}

impl std::error::Error for BackendError {}

/// The built-in backend table: the integrations Bridge has proved, and nothing
/// else. The fourth column is the `transport` its
/// [`builtin_compatibility::BuiltInAgentContract`] declares, carried here so
/// `the_backend_table_and_the_built_in_contracts_cannot_drift` can compare the
/// two rather than trust that they agree.
const BUILT_IN_BACKENDS: &[(&str, &str, BackendKind, &str)] = &[
    (
        "claude",
        "claude.agent-sdk",
        BackendKind::SdkSidecar,
        "claude_agent_sdk_sidecar",
    ),
    (
        "codex",
        "codex.app-server",
        BackendKind::StructuredServer,
        "codex_app_server_stdio",
    ),
    (
        "cursor",
        "cursor.acp",
        BackendKind::Acp,
        "cursor_agent_acp_stdio",
    ),
    (
        "opencode",
        "opencode.server",
        BackendKind::StructuredServer,
        "opencode_authenticated_loopback_http",
    ),
];

/// Every backend Bridge knows how to reach, grouped by the agent it serves.
#[derive(Debug, Default, Clone)]
pub struct BackendResolver {
    candidates: BTreeMap<AgentId, Vec<BackendCandidate>>,
}

impl BackendResolver {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Every proven integration, each bound to the adapter that has been
    /// running it. One candidate per agent today — the point of the
    /// structure is that a second one does not require changing it.
    pub fn built_in() -> Self {
        let mut resolver = Self::empty();
        for (agent, backend, kind, _) in BUILT_IN_BACKENDS {
            let agent = AgentId::parse(agent).expect("built-in agent id is valid");
            resolver
                .register(
                    &agent,
                    BackendCandidate {
                        backend: BackendId::parse(backend).expect("built-in backend id is valid"),
                        // The adapter key is the agent id today, because there
                        // is one adapter per agent. It is a separate field so
                        // the second candidate does not have to lie about it.
                        adapter_id: agent.as_str().to_owned(),
                        kind: *kind,
                    },
                )
                .expect("the built-in table has no duplicate backends");
        }
        resolver
    }

    /// Add a way to reach an agent. Several may coexist; the same [`BackendId`]
    /// twice for one agent may not, because `(agent, backend)` is what a session
    /// binding names and it has to identify exactly one thing.
    ///
    /// The same backend id under a *different* agent is allowed: a shared driver
    /// serving two agents is a shape the identity type deliberately permits.
    pub fn register(
        &mut self,
        agent: &AgentId,
        candidate: BackendCandidate,
    ) -> Result<(), BackendError> {
        let candidates = self.candidates.entry(agent.clone()).or_default();
        if candidates
            .iter()
            .any(|existing| existing.backend == candidate.backend)
        {
            return Err(BackendError::DuplicateBackend {
                agent: agent.as_str().to_owned(),
                backend: candidate.backend.as_str().to_owned(),
            });
        }
        candidates.push(candidate);
        // Policy order, then backend id, so the preferred candidate is a
        // property of the table rather than of registration order.
        candidates.sort_by(|left, right| {
            left.kind
                .preference()
                .cmp(&right.kind.preference())
                .then_with(|| left.backend.cmp(&right.backend))
        });
        Ok(())
    }

    /// Every way to reach this agent, strongest first.
    pub fn candidates(&self, agent: &AgentId) -> &[BackendCandidate] {
        self.candidates
            .get(agent)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// The agents this resolver can reach at all.
    pub fn agents(&self) -> impl Iterator<Item = &AgentId> {
        self.candidates.keys()
    }

    /// One exact candidate. The lookup a resume uses: a bound session asks for
    /// the backend it recorded, never for whichever is preferred now.
    pub fn candidate(&self, agent: &AgentId, backend: &BackendId) -> Option<&BackendCandidate> {
        self.candidates(agent)
            .iter()
            .find(|candidate| &candidate.backend == backend)
    }

    /// What a *fresh* start would choose for this agent.
    pub fn preferred(&self, agent: &AgentId) -> Result<&BackendCandidate, BackendError> {
        self.candidates(agent)
            .first()
            .ok_or_else(|| BackendError::UnknownAgent {
                agent: agent.as_str().to_owned(),
            })
    }

    /// The binding a new session gets: the preferred backend, plus what the
    /// installed copy reports about itself.
    pub fn resolve(
        &self,
        agent: &AgentId,
        backing: &BackendBacking,
    ) -> Result<BackendBinding, BackendError> {
        let candidate = self.preferred(agent)?;
        Ok(BackendBinding {
            agent: agent.clone(),
            backend: candidate.backend.clone(),
            version: backing.version.clone(),
            installation: backing.installation.clone(),
        })
    }

    /// Whether any agent in the table is served by this backend. Used to tell
    /// a cross-switch leftover (backend real, agent wrong) from a build that
    /// genuinely lost the backend — the two must not share an outcome.
    pub fn serves_any_agent(&self, backend: &BackendId) -> bool {
        self.candidates
            .values()
            .flatten()
            .any(|candidate| &candidate.backend == backend)
    }

    /// A stronger candidate than the one a session is bound to, when there is
    /// one. Purely informational: it is what a caller shows to offer the move,
    /// and it never moves anything by itself.
    pub fn preferred_elsewhere(&self, binding: &BackendBinding) -> Option<&BackendCandidate> {
        let preferred = self.preferred(&binding.agent).ok()?;
        (preferred.backend != binding.backend).then_some(preferred)
    }

    /// Resolve one exact backend a session already recorded. Fails with
    /// [`BackendError::BackendUnavailable`] when this build cannot reach it —
    /// which is a resume failure, never a reason to hide the session.
    pub fn resolve_bound(
        &self,
        binding: &BackendBinding,
    ) -> Result<&BackendCandidate, BackendError> {
        self.candidate(&binding.agent, &binding.backend)
            .ok_or_else(|| BackendError::BackendUnavailable {
                agent: binding.agent.as_str().to_owned(),
                backend: binding.backend.as_str().to_owned(),
                version: binding.version.as_ref().map(|v| v.as_str().to_owned()),
            })
    }
}

/// What storage says about a session's backend. Total, because a session that
/// cannot be interpreted must still list and replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredBinding {
    /// No backend was recorded. Either the row predates migration 22, or it was
    /// written by a path that has not started an adapter yet. It is bound on its
    /// next successful start, and it is **not** a backend change.
    Unbound,
    Bound(BackendBinding),
    /// A recorded value this build cannot interpret — a row written by a newer
    /// Bridge, or a corrupted one. Kept raw so the session still reads under its
    /// own name, and never silently rebound: resuming it fails, which is the
    /// legible outcome. The same choice `Harness::Unknown` already makes.
    Unreadable {
        raw: String,
        reason: String,
    },
}

impl StoredBinding {
    pub fn bound(&self) -> Option<&BackendBinding> {
        match self {
            Self::Bound(binding) => Some(binding),
            Self::Unbound | Self::Unreadable { .. } => None,
        }
    }
}

/// Read a session's binding. Never fails on a value it cannot parse — that is
/// [`StoredBinding::Unreadable`], not an error, because a database read that
/// refuses to return makes history unreadable rather than a resume unsafe.
pub fn read_binding(
    db: &rusqlite::Connection,
    session_id: &str,
) -> Result<StoredBinding, BridgeError> {
    /// `(harness, backend_id, backend_version, backend_installation_id)`.
    type BindingRow = (String, Option<String>, Option<String>, Option<String>);
    let row: Option<BindingRow> = db
        .query_row(
            "SELECT harness,backend_id,backend_version,backend_installation_id
             FROM sessions WHERE id=?1",
            rusqlite::params![session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let Some((harness, backend, version, installation)) = row else {
        return Ok(StoredBinding::Unbound);
    };
    let Some(backend_raw) = backend else {
        return Ok(StoredBinding::Unbound);
    };

    let unreadable = |raw: &str, reason: String| StoredBinding::Unreadable {
        raw: raw.to_owned(),
        reason,
    };
    let agent = match AgentId::parse(&harness) {
        Ok(agent) => agent,
        Err(error) => return Ok(unreadable(&harness, error.to_string())),
    };
    let backend = match BackendId::parse(&backend_raw) {
        Ok(backend) => backend,
        Err(error) => return Ok(unreadable(&backend_raw, error.to_string())),
    };
    let version = match version.as_deref().map(BackendVersion::parse).transpose() {
        Ok(version) => version,
        Err(error) => return Ok(unreadable(&backend_raw, error.to_string())),
    };
    let installation = match installation
        .as_deref()
        .map(InstallationId::parse)
        .transpose()
    {
        Ok(installation) => installation,
        Err(error) => return Ok(unreadable(&backend_raw, error.to_string())),
    };
    Ok(StoredBinding::Bound(BackendBinding {
        agent,
        backend,
        version,
        installation,
    }))
}

/// Record which backend served a session. Called on every successful start and
/// resume, so an unbound legacy row acquires its binding the first time it runs
/// under a build that has one.
pub fn write_binding(
    db: &rusqlite::Connection,
    session_id: &str,
    binding: &BackendBinding,
) -> Result<(), BridgeError> {
    db.execute(
        "UPDATE sessions SET backend_id=?2,backend_version=?3,backend_installation_id=?4
         WHERE id=?1",
        rusqlite::params![
            session_id,
            binding.backend.as_str(),
            binding.version.as_ref().map(BackendVersion::as_str),
            binding.installation.as_ref().map(InstallationId::as_str),
        ],
    )?;
    Ok(())
}

/// An explicit authorization to continue one session under a different backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendChangeAuthorization {
    pub from_backend: BackendId,
    pub to_backend: BackendId,
    pub to_version: Option<BackendVersion>,
}

impl BackendChangeAuthorization {
    /// Whether this authorization covers exactly the transition being
    /// attempted. Deliberately exact on both ends: an authorization to move
    /// *from* a backend that is no longer the recorded one is stale, and one
    /// *to* a different target is not the change the user agreed to.
    pub fn permits(&self, from: &BackendBinding, to: &BackendBinding) -> bool {
        self.from_backend == from.backend && self.to_backend == to.backend
    }
}

/// Record an authorization for one session. Replaces any pending one — a user
/// who authorizes a second, different change means the second one.
pub fn authorize_backend_change(
    db: &rusqlite::Connection,
    session_id: &str,
    from: &BackendBinding,
    to: &BackendBinding,
) -> Result<(), BridgeError> {
    db.execute(
        "INSERT INTO backend_change_authorizations
            (session_id,from_backend,to_backend,to_version,authorized_at)
         VALUES(?1,?2,?3,?4,?5)
         ON CONFLICT(session_id) DO UPDATE SET
            from_backend=excluded.from_backend,
            to_backend=excluded.to_backend,
            to_version=excluded.to_version,
            authorized_at=excluded.authorized_at",
        rusqlite::params![
            session_id,
            from.backend.as_str(),
            to.backend.as_str(),
            to.version.as_ref().map(BackendVersion::as_str),
            chrono::Utc::now().to_rfc3339(),
        ],
    )?;
    crate::store::event(
        db,
        "backend",
        "backend.change_authorized",
        session_id,
        &format!("{} -> {}", from.backend, to.backend),
    )?;
    Ok(())
}

/// The pending authorization for a session, if any. An unparseable row reads as
/// no authorization: failing closed here refuses a change, which is the safe
/// direction.
pub fn pending_authorization(
    db: &rusqlite::Connection,
    session_id: &str,
) -> Result<Option<BackendChangeAuthorization>, BridgeError> {
    let row: Option<(String, String, Option<String>)> = db
        .query_row(
            "SELECT from_backend,to_backend,to_version FROM backend_change_authorizations
             WHERE session_id=?1",
            rusqlite::params![session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((from, to, version)) = row else {
        return Ok(None);
    };
    let (Ok(from_backend), Ok(to_backend)) = (BackendId::parse(&from), BackendId::parse(&to))
    else {
        return Ok(None);
    };
    Ok(Some(BackendChangeAuthorization {
        from_backend,
        to_backend,
        to_version: version
            .as_deref()
            .and_then(|v| BackendVersion::parse(v).ok()),
    }))
}

/// Spend the authorization for a session. One-shot: once a change is applied the
/// session's binding names the new backend, so a later resume matches and needs
/// no authorization at all. Leaving it behind would silently permit a second.
pub fn consume_authorization(
    db: &rusqlite::Connection,
    session_id: &str,
) -> Result<(), BridgeError> {
    db.execute(
        "DELETE FROM backend_change_authorizations WHERE session_id=?1",
        rusqlite::params![session_id],
    )?;
    Ok(())
}

/// How a session may continue, given what it recorded and what is installed now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Continuation {
    /// Nothing was recorded — a legacy row, or a session that never launched.
    /// It binds now and runs; this is not a backend change.
    BindFresh(BackendBinding),
    /// The same backend and the same copy of it.
    Resume(BackendBinding),
    /// The same backend, a different version or a different installed copy.
    /// Proceeds, rebinds, and leaves a durable record naming both.
    ResumeMoved {
        binding: BackendBinding,
        previous: BackendBinding,
    },
    /// A different backend, authorized for exactly this transition.
    ResumeRebound {
        binding: BackendBinding,
        previous: BackendBinding,
    },
}

impl Continuation {
    pub fn binding(&self) -> &BackendBinding {
        match self {
            Self::BindFresh(binding)
            | Self::Resume(binding)
            | Self::ResumeMoved { binding, .. }
            | Self::ResumeRebound { binding, .. } => binding,
        }
    }
}

/// Decide how a session continues. Pure: it reads nothing and writes nothing, so
/// every rule below is testable without a database or an installed agent.
///
/// The asymmetry between a moved version and a changed backend is the whole
/// rule. A version moving is the ordinary consequence of a vendor shipping —
/// refusing it would break resume on every update — so it proceeds and is
/// recorded. A backend changing means a different implementation would answer
/// for the same history, which is a decision only a user can make.
pub fn plan_continuation(
    resolver: &BackendResolver,
    stored: &StoredBinding,
    agent: &AgentId,
    backing: &BackendBacking,
    authorization: Option<&BackendChangeAuthorization>,
) -> Result<Continuation, BackendError> {
    let stored = match stored {
        StoredBinding::Unreadable { raw, reason } => {
            return Err(BackendError::BindingUnreadable {
                raw: raw.clone(),
                reason: reason.clone(),
            })
        }
        StoredBinding::Unbound => {
            return Ok(Continuation::BindFresh(resolver.resolve(agent, backing)?))
        }
        StoredBinding::Bound(binding) => binding,
    };

    // The recorded backend has to still exist. Falling through to another
    // candidate when it does not would be exactly the silent substitution this
    // module exists to prevent. One reading is healed rather than refused: a
    // backend that is registered — but under a *different* agent — is a
    // leftover from a harness switch that did not clear the binding columns
    // (`read_binding` composes the agent from the session's current harness,
    // so the stale backend id lands under the new agent's name). There is
    // nothing to substitute: that backend never served this agent, the switch
    // already discarded the provider session, and a fresh bind is exactly what
    // a correctly-cleared row would produce. A backend no agent has is still
    // the refusal — that is a build that lost the backend, the case this
    // module exists for.
    if resolver.resolve_bound(stored).is_err() {
        if resolver.serves_any_agent(&stored.backend) {
            return Ok(Continuation::BindFresh(resolver.resolve(agent, backing)?));
        }
        resolver.resolve_bound(stored)?;
    }

    // A session moves backend only when a user authorizes that exact move. A
    // stronger candidate appearing is reported by `preferred_elsewhere`, not
    // acted on here: acting on it would make registering a backend break every
    // session already running under the old one.
    if let Some(authorization) = authorization {
        let target = resolver
            .candidate(agent, &authorization.to_backend)
            .ok_or_else(|| BackendError::BackendUnavailable {
                agent: agent.as_str().to_owned(),
                backend: authorization.to_backend.as_str().to_owned(),
                version: None,
            })?;
        let rebound = BackendBinding {
            agent: agent.clone(),
            backend: target.backend.clone(),
            version: backing.version.clone(),
            installation: backing.installation.clone(),
        };
        if !stored.same_backend(&rebound) && authorization.permits(stored, &rebound) {
            return Ok(Continuation::ResumeRebound {
                binding: rebound,
                previous: stored.clone(),
            });
        }
    }

    let next = BackendBinding {
        agent: stored.agent.clone(),
        backend: stored.backend.clone(),
        // A backing that reports no version does not erase the one already
        // known. "Not reported" is not "changed" — an external runtime that
        // has never been probed would otherwise blank the record every launch.
        version: backing.version.clone().or_else(|| stored.version.clone()),
        // An installation, unlike a version, means something by its absence:
        // `None` says Bridge owns no payload for this agent now, which is a
        // real transition from managed to external and must be recorded.
        installation: backing.installation.clone(),
    };
    if &next == stored {
        Ok(Continuation::Resume(next))
    } else {
        Ok(Continuation::ResumeMoved {
            binding: next,
            previous: stored.clone(),
        })
    }
}

/// How an absent version reads in a durable record: the backing did not report
/// one, which is a different fact from it having changed.
fn version_or_unreported(version: Option<&BackendVersion>) -> &str {
    version.map_or("unreported", BackendVersion::as_str)
}

/// How an absent installation reads: Bridge owns no payload for this agent,
/// which is a statement rather than a gap.
fn installation_or_none(installation: Option<&InstallationId>) -> &str {
    installation.map_or("none", InstallationId::as_str)
}

/// Apply a planned continuation: write the binding, and leave a durable record
/// of anything that moved. Separated from [`plan_continuation`] so the rule
/// stays pure and only this half needs a database.
pub fn apply_continuation(
    db: &rusqlite::Connection,
    session_id: &str,
    continuation: &Continuation,
) -> Result<(), BridgeError> {
    write_binding(db, session_id, continuation.binding())?;
    match continuation {
        Continuation::BindFresh(_) | Continuation::Resume(_) => {}
        Continuation::ResumeMoved { binding, previous } => {
            // Record what actually moved, not whichever dimension is easier to
            // format. A `ResumeMoved` is produced by full binding inequality,
            // so it covers a version bump, a change of installed copy, or both
            // — and a version line reading `0.147.0 -> 0.147.0` for a payload
            // that was uninstalled would name nothing that happened.
            if previous.version != binding.version {
                crate::store::event(
                    db,
                    "backend",
                    "backend.version_changed",
                    session_id,
                    &format!(
                        "{} {} -> {}",
                        binding.backend,
                        version_or_unreported(previous.version.as_ref()),
                        version_or_unreported(binding.version.as_ref()),
                    ),
                )?;
            }
            if previous.installation != binding.installation {
                crate::store::event(
                    db,
                    "backend",
                    "backend.installation_changed",
                    session_id,
                    &format!(
                        "{} {} -> {}",
                        binding.backend,
                        installation_or_none(previous.installation.as_ref()),
                        installation_or_none(binding.installation.as_ref()),
                    ),
                )?;
            }
        }
        Continuation::ResumeRebound { binding, previous } => {
            crate::store::event(
                db,
                "backend",
                "backend.changed",
                session_id,
                &format!("{} -> {}", previous.backend, binding.backend),
            )?;
            // The authorization covered this one transition and is now spent.
            consume_authorization(db, session_id)?;
        }
    }
    Ok(())
}

/// A launch that has been decided but not yet recorded.
///
/// The two halves are separate because the session row is written *after* the
/// adapter starts: a fresh session has no row to update while it is being
/// decided. So [`plan_launch`] refuses before any process is spawned, and
/// [`LaunchPlan::commit`] records the binding once there is a row to hold it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    /// The adapter registry key to dispatch to.
    pub adapter_id: String,
    /// `None` for an agent the resolver does not know, whose launch is
    /// unchanged and unrecorded.
    continuation: Option<Continuation>,
}

impl LaunchPlan {
    pub fn continuation(&self) -> Option<&Continuation> {
        self.continuation.as_ref()
    }

    /// Record the binding and anything that moved. Call once the session row
    /// exists; a no-op for an agent with no backend.
    pub fn commit(&self, db: &rusqlite::Connection, session_id: &str) -> Result<(), BridgeError> {
        match &self.continuation {
            Some(continuation) => apply_continuation(db, session_id, continuation),
            None => Ok(()),
        }
    }
}

/// Decide which adapter may serve this session, and refuse before anything
/// starts if it may not.
///
/// The one choke point every launch goes through, so a binding cannot be
/// enforced on one path and skipped on another. Returns the same adapter key as
/// before for every agent the resolver does not know, which is how `shell` and
/// any agent without a backend row keep working untouched.
pub fn plan_launch(
    db: &rusqlite::Connection,
    resolver: &BackendResolver,
    session_id: &str,
    harness: &str,
    backing: &BackendBacking,
) -> Result<LaunchPlan, BridgeError> {
    // An agent this build has no backend table for is left exactly as it was.
    // Binding it would mean inventing a backend id for an implementation
    // nothing here describes.
    let unbound = || LaunchPlan {
        adapter_id: harness.to_owned(),
        continuation: None,
    };
    let Ok(agent) = AgentId::parse(harness) else {
        return Ok(unbound());
    };
    if resolver.candidates(&agent).is_empty() {
        return Ok(unbound());
    }

    let stored = read_binding(db, session_id)?;
    let authorization = pending_authorization(db, session_id)?;
    let continuation =
        plan_continuation(resolver, &stored, &agent, backing, authorization.as_ref())
            .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let adapter_id = resolver
        .resolve_bound(continuation.binding())
        .map_err(|error| BridgeError::Invalid(error.to_string()))?
        .adapter_id
        .clone();
    Ok(LaunchPlan {
        adapter_id,
        continuation: Some(continuation),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin_compatibility;

    fn agent(value: &str) -> AgentId {
        AgentId::parse(value).unwrap()
    }

    fn backend(value: &str) -> BackendId {
        BackendId::parse(value).unwrap()
    }

    fn candidate(id: &str, kind: BackendKind) -> BackendCandidate {
        BackendCandidate {
            backend: backend(id),
            adapter_id: id.split('.').next().unwrap().to_owned(),
            kind,
        }
    }

    #[test]
    fn a_resolver_holds_two_backends_for_one_agent_without_collision() {
        let mut resolver = BackendResolver::empty();
        let gemini = agent("gemini");
        // Registered weakest first, so the ordering assertion is about policy
        // rather than about insertion.
        resolver
            .register(&gemini, candidate("gemini.cli", BackendKind::StructuredCli))
            .unwrap();
        resolver
            .register(&gemini, candidate("gemini.acp", BackendKind::Acp))
            .unwrap();

        let candidates = resolver.candidates(&gemini);
        assert_eq!(candidates.len(), 2, "one agent holds two backends");
        assert_eq!(candidates[0].backend, backend("gemini.acp"), "policy order");
        assert_eq!(candidates[1].backend, backend("gemini.cli"));
        assert_eq!(
            resolver.preferred(&gemini).unwrap().backend,
            backend("gemini.acp")
        );

        let duplicate = resolver
            .register(&gemini, candidate("gemini.acp", BackendKind::Acp))
            .unwrap_err();
        assert_eq!(duplicate.code(), "duplicate_backend");
        assert_eq!(
            resolver.candidates(&gemini).len(),
            2,
            "refused, not appended"
        );

        // The same backend id serving a different agent is a permitted shape.
        resolver
            .register(
                &agent("gemini-pro"),
                candidate("gemini.acp", BackendKind::Acp),
            )
            .unwrap();
        assert_eq!(resolver.candidates(&agent("gemini-pro")).len(), 1);
    }

    #[test]
    fn built_in_agents_resolve_to_the_proven_integrations() {
        let resolver = BackendResolver::built_in();
        let expected = [
            ("claude", "claude.agent-sdk", BackendKind::SdkSidecar),
            ("codex", "codex.app-server", BackendKind::StructuredServer),
            ("cursor", "cursor.acp", BackendKind::Acp),
            ("opencode", "opencode.server", BackendKind::StructuredServer),
        ];
        assert_eq!(
            resolver.agents().count(),
            expected.len(),
            "the marketplace adds no agent here"
        );
        for (id, backend_id, kind) in expected {
            let candidates = resolver.candidates(&agent(id));
            assert_eq!(candidates.len(), 1, "{id} has exactly one proven backend");
            assert_eq!(candidates[0].backend, backend(backend_id));
            assert_eq!(candidates[0].kind, kind);
            assert_eq!(
                candidates[0].adapter_id, id,
                "{id} dispatches to its adapter"
            );
        }
    }

    #[test]
    fn every_built_in_backend_names_a_registered_adapter() {
        let registry = crate::adapters::AdapterRegistry::built_in().unwrap();
        let registered: Vec<String> = registry
            .descriptors()
            .into_iter()
            .map(|descriptor| descriptor.id)
            .collect();
        let resolver = BackendResolver::built_in();
        for candidate in resolver
            .agents()
            .flat_map(|agent| resolver.candidates(agent))
        {
            assert!(
                registered.contains(&candidate.adapter_id),
                "{} names adapter {}, which is not registered",
                candidate.backend,
                candidate.adapter_id
            );
        }
    }

    #[test]
    fn the_backend_table_and_the_built_in_contracts_cannot_drift() {
        let contracts = builtin_compatibility::built_in_agent_contracts();
        assert_eq!(
            contracts.len(),
            BUILT_IN_BACKENDS.len(),
            "every built-in contract needs exactly one backend and vice versa"
        );
        for contract in contracts {
            let row = BUILT_IN_BACKENDS
                .iter()
                .find(|(agent, ..)| *agent == contract.id)
                .unwrap_or_else(|| panic!("{} has no backend row", contract.id));
            assert_eq!(
                row.3, contract.transport,
                "{} declares transport {} but its backend row says {}",
                contract.id, contract.transport, row.3
            );
        }
        for (agent, ..) in BUILT_IN_BACKENDS {
            assert!(
                contracts.iter().any(|contract| contract.id == *agent),
                "{agent} has a backend row but no built-in contract"
            );
        }
    }

    #[test]
    fn resolve_through_a_missing_backend_fails_legibly() {
        let resolver = BackendResolver::built_in();
        let binding = BackendBinding {
            agent: agent("codex"),
            backend: backend("codex.acp"),
            version: Some(BackendVersion::parse("0.147.0").unwrap()),
            installation: None,
        };
        let error = resolver.resolve_bound(&binding).unwrap_err();
        assert_eq!(error.code(), "backend_unavailable");
        let message = error.to_string();
        for part in ["codex", "codex.acp", "0.147.0"] {
            assert!(message.contains(part), "{message:?} must name {part}");
        }

        let unknown = resolver.preferred(&agent("gemini")).unwrap_err();
        assert_eq!(unknown.code(), "unknown_agent");
        assert_ne!(
            unknown.code(),
            error.code(),
            "distinct conditions, distinct codes"
        );
    }

    #[test]
    fn resolving_a_fresh_start_carries_the_backing_through() {
        let resolver = BackendResolver::built_in();
        let backing = BackendBacking {
            version: Some(BackendVersion::parse("0.3.209").unwrap()),
            installation: Some(InstallationId::parse("3f9a0c1b7e2d4856af01bc93").unwrap()),
        };
        let binding = resolver.resolve(&agent("claude"), &backing).unwrap();
        assert_eq!(binding.backend, backend("claude.agent-sdk"));
        assert_eq!(binding.version, backing.version);
        assert_eq!(binding.installation, backing.installation);

        // An external runtime reports neither, and that is representable.
        let external = resolver
            .resolve(&agent("codex"), &BackendBacking::default())
            .unwrap();
        assert_eq!(external.backend, backend("codex.app-server"));
        assert!(external.version.is_none() && external.installation.is_none());
    }

    #[test]
    fn same_backend_ignores_version_and_installation() {
        let one = BackendBinding {
            agent: agent("claude"),
            backend: backend("claude.agent-sdk"),
            version: Some(BackendVersion::parse("0.3.209").unwrap()),
            installation: None,
        };
        let moved = BackendBinding {
            version: Some(BackendVersion::parse("0.3.210").unwrap()),
            installation: Some(InstallationId::parse("3f9a0c1b7e2d4856af01bc93").unwrap()),
            ..one.clone()
        };
        let replaced = BackendBinding {
            backend: backend("claude.acp"),
            ..one.clone()
        };
        assert!(
            one.same_backend(&moved),
            "a version bump is the same backend"
        );
        assert!(!one.same_backend(&replaced), "a different backend is not");
    }

    /// A store with one session row, migrated to the current schema.
    fn store_with_session(harness: &str) -> rusqlite::Connection {
        let db = crate::store::open(std::path::Path::new(":memory:")).unwrap();
        // A repo-optional direct chat: no workspace, which migration 7 made
        // legal and which keeps this fixture to the one table under test.
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind)
             VALUES('s',NULL,?1,'S','idle','reported','direct')",
            rusqlite::params![harness],
        )
        .unwrap();
        db
    }

    /// Every durable backend fact a test has provoked, in order, as
    /// `(kind, body)`. The whole record — so a test that expects one event also
    /// fails when a second one it did not ask for appears.
    fn recorded_backend_events(db: &rusqlite::Connection) -> Vec<(String, String)> {
        let mut statement = db
            .prepare("SELECT kind,body FROM events WHERE source='backend' ORDER BY id")
            .unwrap();
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        rows
    }

    fn binding(agent_id: &str, backend_id: &str, version: Option<&str>) -> BackendBinding {
        BackendBinding {
            agent: agent(agent_id),
            backend: backend(backend_id),
            version: version.map(|v| BackendVersion::parse(v).unwrap()),
            installation: None,
        }
    }

    #[test]
    fn a_binding_round_trips_through_storage() {
        let db = store_with_session("claude");
        let full = BackendBinding {
            installation: Some(InstallationId::parse("3f9a0c1b7e2d4856af01bc93").unwrap()),
            ..binding("claude", "claude.agent-sdk", Some("0.3.209"))
        };
        write_binding(&db, "s", &full).unwrap();
        assert_eq!(read_binding(&db, "s").unwrap(), StoredBinding::Bound(full));

        // An external runtime reports no version and owns no installation, and
        // that has to survive the round trip as absence rather than as "".
        let bare = binding("claude", "claude.agent-sdk", None);
        write_binding(&db, "s", &bare).unwrap();
        let read = read_binding(&db, "s").unwrap();
        assert_eq!(read, StoredBinding::Bound(bare));
        assert!(read.bound().unwrap().version.is_none());
    }

    #[test]
    fn an_unbound_legacy_session_reads_as_unbound_not_as_changed() {
        let db = store_with_session("codex");
        // Exactly what migration 22 leaves behind: the row is untouched.
        let stored = read_binding(&db, "s").unwrap();
        assert_eq!(stored, StoredBinding::Unbound);
        assert!(stored.bound().is_none());

        // And an unknown session id is unbound rather than an error, because a
        // caller asking about a row that is gone is not a storage failure.
        assert_eq!(
            read_binding(&db, "missing").unwrap(),
            StoredBinding::Unbound
        );
    }

    #[test]
    fn a_binding_this_build_cannot_interpret_is_kept_not_guessed() {
        let db = store_with_session("codex");
        // A backend id a newer Bridge could write and this one cannot parse.
        db.execute(
            "UPDATE sessions SET backend_id='Codex::AppServer/2' WHERE id='s'",
            [],
        )
        .unwrap();
        match read_binding(&db, "s").unwrap() {
            StoredBinding::Unreadable { raw, reason } => {
                assert_eq!(raw, "Codex::AppServer/2");
                assert!(!reason.is_empty(), "the reason must say what was wrong");
            }
            other => panic!("expected Unreadable, got {other:?}"),
        }

        // A harness id this build cannot parse is the same situation: the
        // session reads, and nothing about it is guessed.
        let db = store_with_session("acp:gemini");
        db.execute(
            "UPDATE sessions SET backend_id='gemini.acp' WHERE id='s'",
            [],
        )
        .unwrap();
        assert!(matches!(
            read_binding(&db, "s").unwrap(),
            StoredBinding::Unreadable { .. }
        ));
    }

    #[test]
    fn an_authorization_does_not_generalize() {
        let db = store_with_session("claude");
        let from = binding("claude", "claude.agent-sdk", Some("0.3.209"));
        let to = binding("claude", "claude.acp", None);
        assert!(pending_authorization(&db, "s").unwrap().is_none());

        authorize_backend_change(&db, "s", &from, &to).unwrap();
        let authorization = pending_authorization(&db, "s").unwrap().unwrap();
        assert!(authorization.permits(&from, &to));

        // Not a different target,
        let elsewhere = binding("claude", "claude.cli", None);
        assert!(!authorization.permits(&from, &elsewhere));
        // not a different origin,
        let moved_on = binding("claude", "claude.acp", None);
        assert!(!authorization.permits(&moved_on, &to));
        // and not a different session.
        assert!(pending_authorization(&db, "other").unwrap().is_none());

        // One-shot: spending it leaves nothing behind for a second change.
        consume_authorization(&db, "s").unwrap();
        assert!(pending_authorization(&db, "s").unwrap().is_none());
    }

    #[test]
    fn authorizing_a_second_change_replaces_the_first() {
        let db = store_with_session("claude");
        let from = binding("claude", "claude.agent-sdk", None);
        let first = binding("claude", "claude.acp", None);
        let second = binding("claude", "claude.cli", None);
        authorize_backend_change(&db, "s", &from, &first).unwrap();
        authorize_backend_change(&db, "s", &from, &second).unwrap();

        let authorization = pending_authorization(&db, "s").unwrap().unwrap();
        assert!(authorization.permits(&from, &second));
        assert!(
            !authorization.permits(&from, &first),
            "the superseded authorization must not still permit its target"
        );

        // Both authorizations are on the durable record, in order.
        let mut statement = db
            .prepare("SELECT body FROM events WHERE kind='backend.change_authorized' ORDER BY id")
            .unwrap();
        let recorded: Vec<String> = statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            recorded,
            vec![
                "claude.agent-sdk -> claude.acp".to_string(),
                "claude.agent-sdk -> claude.cli".to_string()
            ]
        );
    }

    /// A resolver where `claude` has moved to a second, preferred backend, so
    /// a session bound to the SDK sidecar is facing a real change.
    fn resolver_with_claude_moved() -> BackendResolver {
        let mut resolver = BackendResolver::empty();
        let claude = agent("claude");
        resolver
            .register(&claude, candidate("claude.agent-sdk", BackendKind::Acp))
            .unwrap();
        resolver
            .register(
                &claude,
                BackendCandidate {
                    backend: backend("claude.next-sdk"),
                    // A distinct adapter key, so which backend actually
                    // dispatched is observable rather than coincidental.
                    adapter_id: "claude-next".into(),
                    kind: BackendKind::SdkSidecar,
                },
            )
            .unwrap();
        resolver
    }

    fn backing(version: Option<&str>, installation: Option<&str>) -> BackendBacking {
        BackendBacking {
            version: version.map(|v| BackendVersion::parse(v).unwrap()),
            installation: installation.map(|i| InstallationId::parse(i).unwrap()),
        }
    }

    #[test]
    fn an_unbound_session_binds_rather_than_reporting_a_change() {
        let resolver = BackendResolver::built_in();
        let planned = plan_continuation(
            &resolver,
            &StoredBinding::Unbound,
            &agent("codex"),
            &backing(Some("0.147.0"), None),
            None,
        )
        .unwrap();
        assert!(matches!(planned, Continuation::BindFresh(_)));
        assert_eq!(planned.binding().backend, backend("codex.app-server"));
    }

    #[test]
    fn an_unchanged_session_resumes_through_its_recorded_backend() {
        let resolver = BackendResolver::built_in();
        let stored = StoredBinding::Bound(binding("codex", "codex.app-server", Some("0.147.0")));
        let planned = plan_continuation(
            &resolver,
            &stored,
            &agent("codex"),
            &backing(Some("0.147.0"), None),
            None,
        )
        .unwrap();
        assert!(matches!(planned, Continuation::Resume(_)));
    }

    #[test]
    fn a_version_change_resumes_and_records_the_change() {
        let resolver = BackendResolver::built_in();
        let stored = binding("codex", "codex.app-server", Some("0.147.0"));
        let planned = plan_continuation(
            &resolver,
            &StoredBinding::Bound(stored.clone()),
            &agent("codex"),
            &backing(Some("0.148.0"), None),
            None,
        )
        .unwrap();
        match &planned {
            Continuation::ResumeMoved { binding, previous } => {
                assert_eq!(binding.version.as_ref().unwrap().as_str(), "0.148.0");
                assert_eq!(previous.version.as_ref().unwrap().as_str(), "0.147.0");
            }
            other => panic!("expected ResumeMoved, got {other:?}"),
        }

        let db = store_with_session("codex");
        write_binding(&db, "s", &stored).unwrap();
        apply_continuation(&db, "s", &planned).unwrap();
        assert_eq!(
            read_binding(&db, "s").unwrap(),
            StoredBinding::Bound(planned.binding().clone()),
            "the stored version must be updated, not merely reported"
        );
        let recorded: String = db
            .query_row(
                "SELECT body FROM events WHERE kind='backend.version_changed'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(recorded, "codex.app-server 0.147.0 -> 0.148.0");
    }

    #[test]
    fn a_backing_that_reports_no_version_does_not_erase_the_recorded_one() {
        // An external runtime Bridge has not probed reports nothing. Blanking
        // the record every launch would turn "unknown" into "changed" forever.
        let resolver = BackendResolver::built_in();
        let stored = binding("codex", "codex.app-server", Some("0.147.0"));
        let planned = plan_continuation(
            &resolver,
            &StoredBinding::Bound(stored.clone()),
            &agent("codex"),
            &BackendBacking::default(),
            None,
        )
        .unwrap();
        assert_eq!(planned, Continuation::Resume(stored));
    }

    #[test]
    fn losing_the_managed_payload_is_recorded_as_a_move() {
        // Absence of an installation, unlike absence of a version, is a fact:
        // Bridge owns nothing for this agent now.
        let resolver = BackendResolver::built_in();
        let stored = BackendBinding {
            installation: Some(InstallationId::parse("3f9a0c1b7e2d4856af01bc93").unwrap()),
            ..binding("codex", "codex.app-server", Some("0.147.0"))
        };
        let planned = plan_continuation(
            &resolver,
            &StoredBinding::Bound(stored.clone()),
            &agent("codex"),
            &backing(Some("0.147.0"), None),
            None,
        )
        .unwrap();
        match &planned {
            Continuation::ResumeMoved { binding, .. } => assert!(binding.installation.is_none()),
            other => panic!("expected ResumeMoved, got {other:?}"),
        }

        // And the record has to name what moved. The version did not, so a
        // version line here would read `0.147.0 -> 0.147.0` and describe
        // nothing that happened.
        let db = store_with_session("codex");
        write_binding(&db, "s", &stored).unwrap();
        apply_continuation(&db, "s", &planned).unwrap();
        assert_eq!(
            recorded_backend_events(&db),
            vec![(
                "backend.installation_changed".to_string(),
                "codex.app-server 3f9a0c1b7e2d4856af01bc93 -> none".to_string()
            )],
            "an installation-only move records the installation, and only it"
        );
    }

    #[test]
    fn a_move_of_both_version_and_installation_records_both() {
        let resolver = BackendResolver::built_in();
        let stored = BackendBinding {
            installation: Some(InstallationId::parse("3f9a0c1b7e2d4856af01bc93").unwrap()),
            ..binding("codex", "codex.app-server", Some("0.147.0"))
        };
        let planned = plan_continuation(
            &resolver,
            &StoredBinding::Bound(stored.clone()),
            &agent("codex"),
            &backing(Some("0.148.0"), Some("aa11bb22cc33dd44ee55ff66")),
            None,
        )
        .unwrap();
        let db = store_with_session("codex");
        write_binding(&db, "s", &stored).unwrap();
        apply_continuation(&db, "s", &planned).unwrap();
        assert_eq!(
            recorded_backend_events(&db),
            vec![
                (
                    "backend.version_changed".to_string(),
                    "codex.app-server 0.147.0 -> 0.148.0".to_string()
                ),
                (
                    "backend.installation_changed".to_string(),
                    "codex.app-server 3f9a0c1b7e2d4856af01bc93 -> aa11bb22cc33dd44ee55ff66"
                        .to_string()
                ),
            ]
        );
    }

    #[test]
    fn a_newer_preferred_backend_does_not_move_or_block_a_bound_session() {
        // The amended rule. A stronger candidate appearing must not turn every
        // existing session into one that refuses to resume.
        let resolver = resolver_with_claude_moved();
        let stored = binding("claude", "claude.agent-sdk", Some("0.3.209"));
        let planned = plan_continuation(
            &resolver,
            &StoredBinding::Bound(stored.clone()),
            &agent("claude"),
            &backing(Some("0.3.209"), None),
            None,
        )
        .unwrap();
        assert_eq!(
            planned,
            Continuation::Resume(stored.clone()),
            "a bound session sticks to the backend it recorded"
        );

        // The divergence is still visible, so a caller can offer the move.
        assert_eq!(
            resolver.preferred_elsewhere(&stored).unwrap().backend,
            backend("claude.next-sdk")
        );
        // And there is nothing to offer once it is on the preferred one.
        let moved = binding("claude", "claude.next-sdk", None);
        assert!(resolver.preferred_elsewhere(&moved).is_none());
    }

    #[test]
    fn a_backend_change_happens_only_through_an_authorization() {
        let resolver = resolver_with_claude_moved();
        let stored = binding("claude", "claude.agent-sdk", Some("0.3.209"));
        let plan = |authorization: Option<&BackendChangeAuthorization>| {
            plan_continuation(
                &resolver,
                &StoredBinding::Bound(stored.clone()),
                &agent("claude"),
                &backing(Some("0.4.0"), None),
                authorization,
            )
        };

        // An authorization for a backend this agent does not have is refused
        // rather than quietly ignored.
        let absent = BackendChangeAuthorization {
            from_backend: backend("claude.agent-sdk"),
            to_backend: backend("claude.acp"),
            to_version: None,
        };
        assert_eq!(
            plan(Some(&absent)).unwrap_err().code(),
            "backend_unavailable"
        );

        // An authorization whose origin is stale does not move the session.
        let stale = BackendChangeAuthorization {
            from_backend: backend("claude.next-sdk"),
            to_backend: backend("claude.next-sdk"),
            to_version: None,
        };
        assert!(matches!(
            plan(Some(&stale)).unwrap(),
            Continuation::ResumeMoved { .. } | Continuation::Resume(_)
        ));

        let right = BackendChangeAuthorization {
            from_backend: backend("claude.agent-sdk"),
            to_backend: backend("claude.next-sdk"),
            to_version: None,
        };
        match plan(Some(&right)).unwrap() {
            Continuation::ResumeRebound { binding, previous } => {
                assert_eq!(binding.backend, backend("claude.next-sdk"));
                assert_eq!(previous.backend, backend("claude.agent-sdk"));
            }
            other => panic!("expected ResumeRebound, got {other:?}"),
        }
    }

    #[test]
    fn applying_an_authorized_change_spends_the_authorization() {
        let resolver = resolver_with_claude_moved();
        let db = store_with_session("claude");
        let stored = binding("claude", "claude.agent-sdk", Some("0.3.209"));
        write_binding(&db, "s", &stored).unwrap();
        let target = binding("claude", "claude.next-sdk", Some("0.4.0"));
        authorize_backend_change(&db, "s", &stored, &target).unwrap();

        let authorization = pending_authorization(&db, "s").unwrap().unwrap();
        let planned = plan_continuation(
            &resolver,
            &read_binding(&db, "s").unwrap(),
            &agent("claude"),
            &backing(Some("0.4.0"), None),
            Some(&authorization),
        )
        .unwrap();
        apply_continuation(&db, "s", &planned).unwrap();

        assert_eq!(
            read_binding(&db, "s").unwrap().bound().unwrap().backend,
            backend("claude.next-sdk")
        );
        assert!(
            pending_authorization(&db, "s").unwrap().is_none(),
            "the authorization covered one transition and is spent"
        );
        assert_eq!(
            db.query_row(
                "SELECT body FROM events WHERE kind='backend.changed'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "claude.agent-sdk -> claude.next-sdk"
        );

        // And the next resume needs no authorization at all, because the
        // session now names the backend that would be chosen anyway.
        assert!(matches!(
            plan_continuation(
                &resolver,
                &read_binding(&db, "s").unwrap(),
                &agent("claude"),
                &backing(Some("0.4.0"), None),
                None,
            )
            .unwrap(),
            Continuation::Resume(_)
        ));
    }

    #[test]
    fn a_session_whose_backend_is_gone_is_unavailable_not_changed() {
        // The distinction matters: "changed" invites the user to authorize a
        // move, and there is nothing to move to.
        let resolver = BackendResolver::built_in();
        let stored = StoredBinding::Bound(binding("codex", "codex.acp", Some("0.147.0")));
        let error = plan_continuation(
            &resolver,
            &stored,
            &agent("codex"),
            &backing(Some("0.147.0"), None),
            None,
        )
        .unwrap_err();
        assert_eq!(error.code(), "backend_unavailable");
    }

    #[test]
    fn an_uninterpretable_binding_refuses_to_run_and_never_rebinds() {
        let resolver = BackendResolver::built_in();
        let stored = StoredBinding::Unreadable {
            raw: "Codex::AppServer/2".into(),
            reason: "invalid backend id".into(),
        };
        let error = plan_continuation(
            &resolver,
            &stored,
            &agent("codex"),
            &backing(Some("0.147.0"), None),
            None,
        )
        .unwrap_err();
        assert_eq!(error.code(), "binding_unreadable");
        assert!(error.to_string().contains("Codex::AppServer/2"));
    }

    #[test]
    fn every_backend_error_condition_has_its_own_code() {
        let codes = [
            BackendError::UnknownAgent { agent: "a".into() },
            BackendError::BackendUnavailable {
                agent: "a".into(),
                backend: "b".into(),
                version: None,
            },
            BackendError::DuplicateBackend {
                agent: "a".into(),
                backend: "b".into(),
            },
            BackendError::BindingUnreadable {
                raw: "r".into(),
                reason: "why".into(),
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

    #[test]
    fn installation_ids_from_the_payload_engine_parse_as_identities() {
        // The identity type claims the payload engine's grammar. Check it
        // against the real derivation rather than a hand-written example, so a
        // change to either side fails here.
        let derived = crate::managed_payload::installation_id(
            "claude",
            "npm:@anthropic-ai/claude-agent-sdk@0.3.209",
            "darwin-arm64",
            "sha256-0000",
        );
        assert_eq!(
            InstallationId::parse(&derived).unwrap().as_str(),
            derived,
            "a derived installation id must satisfy InstallationId"
        );
        // And the npm-coordinate spelling a recipe pins is a legal version.
        assert!(
            BackendVersion::parse("npm:@anthropic-ai/claude-agent-sdk@0.3.209").is_ok(),
            "the recipe's own version spelling must round-trip"
        );
    }

    /// The exact field failure: switching a chat's harness used to leave the
    /// old backend id in place, and `read_binding` composes the binding's agent
    /// from the *current* harness column — so the row read as
    /// `{agent: codex, backend: claude.agent-sdk}` and every launch refused
    /// with "codex was served by claude.agent-sdk, which this build does not
    /// have". Rows the old bug already wrote must heal to a fresh bind.
    #[test]
    fn a_cross_switch_leftover_binding_heals_to_a_fresh_bind() {
        let resolver = BackendResolver::built_in();
        let stored = StoredBinding::Bound(BackendBinding {
            agent: agent("codex"),
            backend: backend("claude.agent-sdk"),
            version: None,
            installation: None,
        });
        let continuation = plan_continuation(
            &resolver,
            &stored,
            &agent("codex"),
            &BackendBacking::default(),
            None,
        )
        .expect("a leftover from a harness switch is healed, not refused");
        match continuation {
            Continuation::BindFresh(binding) => {
                assert_eq!(binding.agent, agent("codex"));
                assert_eq!(binding.backend, backend("codex.app-server"));
            }
            other => panic!("expected BindFresh, got {other:?}"),
        }

        // End to end: a session row written by the old bug launches again.
        let db = store_with_session("codex");
        db.execute(
            "UPDATE sessions SET backend_id='claude.agent-sdk' WHERE id='s'",
            [],
        )
        .unwrap();
        let plan = plan_launch(&db, &resolver, "s", "codex", &BackendBacking::default())
            .expect("the bricked session launches");
        assert_eq!(plan.adapter_id, "codex");
    }

    /// The heal must not swallow the case the refusal exists for: a backend no
    /// agent in this build has is a lost backend, not a switch leftover.
    #[test]
    fn a_backend_no_agent_serves_still_refuses() {
        let resolver = BackendResolver::built_in();
        let stored = StoredBinding::Bound(BackendBinding {
            agent: agent("codex"),
            backend: backend("codex.acp"),
            version: None,
            installation: None,
        });
        let error = plan_continuation(
            &resolver,
            &stored,
            &agent("codex"),
            &BackendBacking::default(),
            None,
        )
        .unwrap_err();
        assert!(
            matches!(error, BackendError::BackendUnavailable { .. }),
            "a genuinely missing backend is a refusal: {error}"
        );
    }

    #[test]
    fn an_agent_with_no_backend_row_is_left_exactly_as_it_was() {
        // `shell` is a harness, not a marketplace agent: it has no backend and
        // must dispatch to the same adapter key it always did, unbound.
        let db = store_with_session("shell");
        let resolver = BackendResolver::built_in();
        let plan = plan_launch(&db, &resolver, "s", "shell", &BackendBacking::default()).unwrap();
        assert_eq!(plan.adapter_id, "shell");
        assert!(plan.continuation().is_none(), "nothing to record");
        plan.commit(&db, "s").unwrap();
        assert_eq!(read_binding(&db, "s").unwrap(), StoredBinding::Unbound);

        // Same for an id this build cannot even parse.
        let db = store_with_session("acp:gemini");
        let plan = plan_launch(
            &db,
            &resolver,
            "s",
            "acp:gemini",
            &BackendBacking::default(),
        )
        .unwrap();
        assert_eq!(plan.adapter_id, "acp:gemini");
    }

    #[test]
    fn a_launch_binds_the_session_and_dispatches_to_the_backends_adapter() {
        let db = store_with_session("codex");
        let resolver = BackendResolver::built_in();
        let plan = plan_launch(
            &db,
            &resolver,
            "s",
            "codex",
            &backing(Some("0.147.0"), None),
        )
        .unwrap();
        assert_eq!(plan.adapter_id, "codex", "the registry key is unchanged");
        assert_eq!(
            read_binding(&db, "s").unwrap(),
            StoredBinding::Unbound,
            "planning decides; it must not write before the session row exists"
        );
        plan.commit(&db, "s").unwrap();
        assert_eq!(
            read_binding(&db, "s").unwrap(),
            StoredBinding::Bound(binding("codex", "codex.app-server", Some("0.147.0"))),
            "committing the launch is what records the binding"
        );

        // A second launch after a vendor update rebinds rather than refusing.
        plan_launch(
            &db,
            &resolver,
            "s",
            "codex",
            &backing(Some("0.148.0"), None),
        )
        .unwrap()
        .commit(&db, "s")
        .unwrap();
        assert_eq!(
            read_binding(&db, "s")
                .unwrap()
                .bound()
                .unwrap()
                .version
                .as_ref()
                .unwrap()
                .as_str(),
            "0.148.0"
        );
    }

    #[test]
    fn a_launch_dispatches_to_the_bound_backend_not_the_preferred_one() {
        let db = store_with_session("claude");
        let resolver = resolver_with_claude_moved();
        let stored = binding("claude", "claude.agent-sdk", Some("0.3.209"));
        write_binding(&db, "s", &stored).unwrap();

        // `claude.next-sdk` is preferred and would win a fresh resolution. The
        // bound session must still reach the adapter its own backend names.
        let plan = plan_launch(
            &db,
            &resolver,
            "s",
            "claude",
            &backing(Some("0.3.209"), None),
        )
        .unwrap();
        assert_eq!(plan.adapter_id, "claude");
        assert_eq!(
            resolver.preferred(&agent("claude")).unwrap().adapter_id,
            "claude-next",
            "the preferred candidate is genuinely a different adapter"
        );

        // Authorized, the same launch dispatches to the new backend instead.
        let target = binding("claude", "claude.next-sdk", None);
        authorize_backend_change(&db, "s", &stored, &target).unwrap();
        let plan =
            plan_launch(&db, &resolver, "s", "claude", &backing(Some("0.4.0"), None)).unwrap();
        assert_eq!(plan.adapter_id, "claude-next");
        plan.commit(&db, "s").unwrap();
        assert_eq!(
            read_binding(&db, "s").unwrap().bound().unwrap().backend,
            backend("claude.next-sdk")
        );
        assert!(pending_authorization(&db, "s").unwrap().is_none());
    }

    #[test]
    fn a_launch_through_a_backend_this_build_lost_refuses_before_starting() {
        let db = store_with_session("codex");
        let resolver = BackendResolver::built_in();
        write_binding(&db, "s", &binding("codex", "codex.acp", Some("0.147.0"))).unwrap();

        let error = plan_launch(
            &db,
            &resolver,
            "s",
            "codex",
            &backing(Some("0.147.0"), None),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("codex.acp"), "{error}");
        assert_eq!(
            read_binding(&db, "s").unwrap().bound().unwrap().backend,
            backend("codex.acp"),
            "a refused launch must not have substituted another backend"
        );
    }

    #[test]
    fn a_session_whose_backend_is_gone_still_lists_and_replays() {
        // The half of "fails legibly" that is easy to lose: only *resuming* may
        // fail. A session whose backend this build no longer has must still
        // appear, under its own name, with its transcript intact.
        let db = store_with_session("codex");
        write_binding(&db, "s", &binding("codex", "codex.acp", Some("0.147.0"))).unwrap();
        crate::store::append_session_entry(
            &db,
            "s",
            None,
            "message",
            &serde_json::json!({"role": "user", "text": "still readable"}),
            None,
            "visible",
            None,
        )
        .unwrap();

        let state = crate::store::state(&db).unwrap();
        let session = state
            .sessions
            .iter()
            .find(|session| session.id == "s")
            .expect("a session with a lost backend must still list");
        assert_eq!(session.harness, crate::model::Harness::Codex);
        assert_eq!(
            crate::store::session_entries(&db, "s").unwrap().len(),
            1,
            "its transcript must still replay"
        );

        // And only the resume is refused.
        assert!(plan_launch(
            &db,
            &BackendResolver::built_in(),
            "s",
            "codex",
            &BackendBacking::default()
        )
        .is_err());
    }

    #[test]
    fn no_part_of_the_binding_surface_can_carry_a_credential() {
        // The identity scalars are checked in bridge-protocol. This is the
        // composite half: a binding, a candidate, and an authorization together
        // hold no field a token, key, or vendor configuration value could live
        // in. A field added later has to come past this test.
        let candidate = candidate("codex.app-server", BackendKind::StructuredServer);
        let bound = binding("codex", "codex.app-server", Some("0.147.0"));
        let authorization = BackendChangeAuthorization {
            from_backend: backend("codex.app-server"),
            to_backend: backend("codex.acp"),
            to_version: None,
        };
        // Destructured exhaustively and without `..`: adding a field to any of
        // the three makes this a compile error rather than a silent widening.
        let BackendCandidate {
            backend: _,
            adapter_id,
            kind: _,
        } = &candidate;
        let BackendBinding {
            agent,
            backend: _,
            version: _,
            installation: _,
        } = &bound;
        let BackendChangeAuthorization {
            from_backend: _,
            to_backend: _,
            to_version: _,
        } = &authorization;

        // Every field above is either a validated identity scalar or, in the
        // one free-text case, an adapter registry key. That one is the only
        // place a value could hide, so it is the one checked by content.
        for forbidden in ["token", "key", "secret", "password", "credential"] {
            assert!(
                !adapter_id.to_lowercase().contains(forbidden),
                "{forbidden:?} appears in an adapter key: {adapter_id}"
            );
        }
        assert_eq!(agent.as_str(), "codex");
    }

    #[test]
    fn every_adapter_launch_goes_through_the_binding_choke_point() {
        // The binding is only a guarantee if no launch path can skip it. Each
        // of the three flows that reaches the adapter registry must dispatch
        // through a plan, and a fourth added later must too.
        //
        // An earlier version of this test blocklisted four literal spellings,
        // which a new `start(&other_id` or a differently-wrapped call would
        // have walked straight past. This reads the *argument* at every
        // dispatch site instead: whatever a future path is called, the id it
        // dispatches on has to be one that came out of a plan.
        let source = include_str!("live_turn.rs");
        let dispatched = registry_dispatch_arguments(source);
        assert!(
            dispatched.len() >= 7,
            "expected the known dispatch sites; found {dispatched:?}"
        );
        // `launch_adapter_id` is `dispatch_id.clone()`, moved into a closure.
        const FROM_A_PLAN: [&str; 2] = ["dispatch_id", "launch_adapter_id"];
        for argument in &dispatched {
            let name = argument.trim_start_matches('&');
            assert!(
                FROM_A_PLAN.contains(&name),
                "{argument} dispatches on an id no plan_launch produced; every \
                 adapter launch must go through the binding"
            );
        }
        assert_eq!(
            source.matches("backend_binding::plan_launch(").count(),
            3,
            "the three launch flows each plan exactly once — a fourth flow must \
             add its own plan and update this count deliberately"
        );
    }

    /// The first argument of every `AdapterRegistry` launch call in a source
    /// file — `start`, `resume`, and the `supports_native_resume` query that
    /// decides between them, since asking the wrong backend whether it can
    /// resume picks the wrong plan just as surely as launching it would.
    ///
    /// Receiver-based rather than name-based: it matches any expression ending
    /// in `registry`, so both `state.adapter_registry` and the cloned local
    /// `registry` are covered however the call happens to be wrapped.
    fn registry_dispatch_arguments(source: &str) -> Vec<String> {
        const CALLS: [&str; 3] = [".start(", ".resume(", ".supports_native_resume("];
        let mut arguments = Vec::new();
        for call in CALLS {
            for (index, _) in source.match_indices(call) {
                if !source[..index].trim_end().ends_with("registry") {
                    continue;
                }
                let open = index + call.len();
                let rest = source[open..].trim_start();
                let end = rest
                    .find([',', ')'])
                    .unwrap_or_else(|| panic!("unterminated call argument near {rest:.40}"));
                arguments.push(rest[..end].trim().to_owned());
            }
        }
        arguments
    }

    #[test]
    fn the_backend_policy_orders_the_strongest_interface_first() {
        let mut kinds = [
            BackendKind::StructuredCli,
            BackendKind::Acp,
            BackendKind::SdkSidecar,
            BackendKind::StructuredServer,
        ];
        kinds.sort_by_key(|kind| kind.preference());
        assert_eq!(
            kinds,
            [
                BackendKind::SdkSidecar,
                BackendKind::StructuredServer,
                BackendKind::Acp,
                BackendKind::StructuredCli,
            ]
        );
    }
}

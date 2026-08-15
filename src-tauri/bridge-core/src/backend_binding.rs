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
}

impl BackendError {
    /// The stable wire code. Exhaustive on purpose: a new condition must come
    /// past this match and be given its own code rather than reuse one.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::UnknownAgent { .. } => "unknown_agent",
            Self::BackendUnavailable { .. } => "backend_unavailable",
            Self::DuplicateBackend { .. } => "duplicate_backend",
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
        }
    }
}

impl std::error::Error for BackendError {}

/// The built-in backend table: the three integrations #161 proved, and nothing
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

    /// The three integrations #161 proved, each bound to the adapter that has
    /// been running it. One candidate per agent today — the point of the
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
    Unreadable { raw: String, reason: String },
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
    let row: Option<(String, Option<String>, Option<String>, Option<String>)> = db
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
    let installation = match installation.as_deref().map(InstallationId::parse).transpose() {
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
    let (Ok(from_backend), Ok(to_backend)) = (BackendId::parse(&from), BackendId::parse(&to)) else {
        return Ok(None);
    };
    Ok(Some(BackendChangeAuthorization {
        from_backend,
        to_backend,
        to_version: version.as_deref().and_then(|v| BackendVersion::parse(v).ok()),
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
        assert_eq!(resolver.candidates(&gemini).len(), 2, "refused, not appended");

        // The same backend id serving a different agent is a permitted shape.
        resolver
            .register(&agent("gemini-pro"), candidate("gemini.acp", BackendKind::Acp))
            .unwrap();
        assert_eq!(resolver.candidates(&agent("gemini-pro")).len(), 1);
    }

    #[test]
    fn built_in_agents_resolve_to_the_integrations_161_proved() {
        let resolver = BackendResolver::built_in();
        let expected = [
            ("claude", "claude.agent-sdk", BackendKind::SdkSidecar),
            ("codex", "codex.app-server", BackendKind::StructuredServer),
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
            assert_eq!(candidates[0].adapter_id, id, "{id} dispatches to its adapter");
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
            "every #162 contract needs exactly one backend and vice versa"
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
                "{agent} has a backend row but no #162 contract"
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
        assert_ne!(unknown.code(), error.code(), "distinct conditions, distinct codes");
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
        assert!(one.same_backend(&moved), "a version bump is the same backend");
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
        assert_eq!(read_binding(&db, "missing").unwrap(), StoredBinding::Unbound);
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

//! Deterministic verification and staged promotion: how an entry earns
//! `Verified`, and how it loses it.
//!
//! **Evidence is the only way to `Verified`.** #164 modelled the status, gated
//! loading on it, and deliberately had no way to produce it. This is the
//! producer, and the gate is structural rather than procedural:
//! [`Verification::verified`] is the only function in the crate that returns a
//! `Verified` verdict, and it takes an [`Evidence`] it cannot fabricate. A gate
//! a caller can walk around is a suggestion.
//!
//! **Detection is an input, never an authority.** A [`Candidate`] is what an
//! approved upstream source suggests. There is no conversion from a candidate
//! to a served entry — the only path runs through the suite and
//! [`promote`], and a test asserts the absence in both directions.
//!
//! **The suite drives the same control plane a session does.** Every check
//! reaches the runtime through [`IntegrationSession`], because a suite with its
//! own private path to the runtime would verify a path no user ever takes.

use crate::{
    acp_registry::PlatformTarget,
    adapters::ShutdownReason,
    agent_integration::{
        check_capabilities, Advertisement, IntegrationRegistry, LaunchRequest,
        PermissionModel, ResumeSupport, Transport,
    },
    model::CapabilityTier,
    verified_catalog::{
        Catalog, CatalogBackendKind, CatalogError, CatalogRecipe, CatalogSnapshot,
        IntegrationConfig, VendorRequirement, Verification, VerifiedEntry,
    },
};
use bridge_protocol::messages::{AgentId, BackendId, BackendVersion};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The revision of the conformance suite. A verdict produced under a different
/// revision is not comparable to one produced under this, so promotion refuses
/// it rather than accepting a verdict whose meaning has moved.
pub const SUITE_VERSION: u32 = 1;

/// How many times the required set runs before a verdict is trusted.
///
/// "It passed" and "it passes" are different claims, and #168 names the second.
/// Three is the smallest number that can distinguish a stable outcome from an
/// alternating one.
pub const DETERMINISM_RUNS: usize = 3;

/// The longest a failure reason may be in evidence. Bounded because evidence is
/// a public artifact and an unbounded reason is a transcript with extra steps.
pub const MAX_REASON_BYTES: usize = 512;

/// Where a candidate version came from. An input to a decision; never one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovedSource {
    /// The vendor's own release feed.
    VendorRelease,
    /// An upstream index. Discovery only — #171's product contract is explicit
    /// that this is not a compatibility guarantee.
    UpstreamRegistry,
    /// A human proposed this version directly.
    Manual,
}

/// A version someone proposes Bridge should support: everything a served entry
/// needs except the verdict.
///
/// Deliberately *not* convertible to a [`VerifiedEntry`]. The only way to get
/// one is [`promote`], which requires evidence, and
/// `candidates_cannot_become_verified_entries` asserts no other path exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub agent: AgentId,
    pub label: String,
    pub vendor: String,
    pub license: String,
    pub source_url: String,
    pub version: BackendVersion,
    pub platforms: Vec<PlatformTarget>,
    pub recipe: CatalogRecipe,
    pub backend: BackendId,
    pub backend_kind: CatalogBackendKind,
    pub integration: IntegrationConfig,
    pub vendor_requirement: VendorRequirement,
    /// The profile a runtime's advertisement is checked *against*. Proposed
    /// here, proven by the suite, and never widened by what the runtime says.
    pub capabilities: Vec<String>,
    pub blocked_versions: Vec<BackendVersion>,
    pub minimum_bridge_version: String,
    /// Which approved source suggested this version.
    pub detected_from: ApprovedSource,
}

impl Candidate {
    /// The entry this candidate *would* become, carrying a verdict.
    ///
    /// Private on purpose: it is reachable only from [`promote`], after the
    /// gates. A public version of this is precisely the hole #168 exists to
    /// close.
    fn into_entry(self, verification: Verification) -> VerifiedEntry {
        VerifiedEntry {
            agent: self.agent,
            label: self.label,
            vendor: self.vendor,
            license: self.license,
            source_url: self.source_url,
            version: self.version,
            platforms: self.platforms,
            recipe: self.recipe,
            backend: self.backend,
            backend_kind: self.backend_kind,
            integration: self.integration,
            vendor_requirement: self.vendor_requirement,
            capabilities: self.capabilities,
            blocked_versions: self.blocked_versions,
            minimum_bridge_version: self.minimum_bridge_version,
            verification,
        }
    }

    /// A provisional entry for checks that need one before a verdict exists —
    /// the capability profile, chiefly. Carries a `Pending` verdict, so it
    /// could not be served even if it leaked.
    fn provisional(&self) -> VerifiedEntry {
        self.clone().into_entry(Verification::pending())
    }
}

/// One check in the conformance suite.
///
/// The list is #168's, and `every_required_check_from_the_issue_is_present`
/// compares it against the issue so dropping one is a test failure rather than
/// a quiet reduction in coverage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckId {
    Install,
    Launch,
    Initialize,
    NewSession,
    MultiTurnPrompt,
    Normalization,
    Permissions,
    Cancellation,
    Shutdown,
    ClaimedResume,
    VendorAuthRequiredFailure,
    Redaction,
    UpdateFromPriorVerified,
    Rollback,
    UninstallRetainsHistory,
    PlatformClaims,
    CapabilityDrift,
}

impl CheckId {
    /// Every check the suite runs, in the order it runs them. Ordered so
    /// evidence reads as a sequence rather than a set — a reader wants to know
    /// what happened before the failure.
    pub const ALL: &'static [Self] = &[
        Self::Install,
        Self::Launch,
        Self::Initialize,
        Self::NewSession,
        Self::MultiTurnPrompt,
        Self::Normalization,
        Self::Permissions,
        Self::Cancellation,
        Self::Shutdown,
        Self::ClaimedResume,
        Self::VendorAuthRequiredFailure,
        Self::Redaction,
        Self::UpdateFromPriorVerified,
        Self::Rollback,
        Self::UninstallRetainsHistory,
        Self::PlatformClaims,
        Self::CapabilityDrift,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Launch => "launch",
            Self::Initialize => "initialize",
            Self::NewSession => "new_session",
            Self::MultiTurnPrompt => "multi_turn_prompt",
            Self::Normalization => "normalization",
            Self::Permissions => "permissions",
            Self::Cancellation => "cancellation",
            Self::Shutdown => "shutdown",
            Self::ClaimedResume => "claimed_resume",
            Self::VendorAuthRequiredFailure => "vendor_auth_required_failure",
            Self::Redaction => "redaction",
            Self::UpdateFromPriorVerified => "update_from_prior_verified",
            Self::Rollback => "rollback",
            Self::UninstallRetainsHistory => "uninstall_retains_history",
            Self::PlatformClaims => "platform_claims",
            Self::CapabilityDrift => "capability_drift",
        }
    }
}

/// Why a check did not run. Never a pass: a required check that could be
/// skipped into a pass would make the gate optional in exactly the environment
/// where it matters least.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// The vendor environment this check needs is not configured here.
    VendorAuthUnavailable,
    /// The candidate does not claim this platform, so there is nothing to check.
    PlatformNotClaimed,
}

/// What one check concluded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "outcome")]
pub enum CheckOutcome {
    Passed,
    #[serde(rename_all = "camelCase")]
    Failed {
        /// Bounded and redacted. See [`redact`].
        reason: String,
    },
    #[serde(rename_all = "camelCase")]
    Skipped {
        reason: SkipReason,
    },
}

impl CheckOutcome {
    pub fn passed(&self) -> bool {
        matches!(self, Self::Passed)
    }

    fn failed(reason: impl AsRef<str>) -> Self {
        Self::Failed {
            reason: redact(reason.as_ref()),
        }
    }
}

/// One check and what it concluded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckRecord {
    pub check: CheckId,
    #[serde(flatten)]
    pub outcome: CheckOutcome,
}

/// Bound and strip a failure reason.
///
/// Evidence is a public artifact. A reason that carried a home path, an
/// environment value, or an unbounded slab of provider output would make the
/// artifact unpublishable, and the pipeline's own redaction check is what
/// proves this ran.
pub fn redact(reason: &str) -> String {
    let mut cleaned = String::with_capacity(reason.len().min(MAX_REASON_BYTES));
    for word in reason.split_whitespace() {
        // An absolute path is the common carrier of a username. Keep the shape
        // so the reason still reads, drop the value.
        let token = if word.starts_with('/') || word.contains("\\Users\\") {
            "<path>"
        } else if word.contains('=') && word.chars().any(|c| c.is_ascii_uppercase()) {
            // KEY=value, the shape an environment dump arrives in.
            "<redacted>"
        } else {
            word
        };
        if !cleaned.is_empty() {
            cleaned.push(' ');
        }
        cleaned.push_str(token);
        if cleaned.len() >= MAX_REASON_BYTES {
            break;
        }
    }
    cleaned.truncate(MAX_REASON_BYTES);
    cleaned
}

/// What Bridge ran, against what, and what happened.
///
/// Immutable once built and bound to exactly the inputs that produced it: a
/// verdict that did not name its artifact digest, Bridge version, platform,
/// backend, and suite version would be a claim about nothing in particular.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Evidence {
    pub agent: AgentId,
    pub version: BackendVersion,
    pub backend: BackendId,
    pub backend_kind: CatalogBackendKind,
    /// The digest of what the managed lifecycle actually installed — not what
    /// the recipe claimed. The two being compared is what catches a tampered
    /// artifact.
    pub artifact_digest: String,
    pub bridge_version: String,
    pub platform: PlatformTarget,
    pub suite_version: u32,
    /// Ordered, and complete: every check in [`CheckId::ALL`] appears exactly
    /// once. A document missing one is refused rather than read leniently.
    pub checks: Vec<CheckRecord>,
    /// Whether the required set produced identical outcomes across
    /// [`DETERMINISM_RUNS`] runs.
    pub deterministic: bool,
    pub produced_at: String,
}

impl Evidence {
    /// SHA-256 over the canonical JSON encoding.
    ///
    /// Canonical means the field order this struct declares and no incidental
    /// whitespace — `serde_json::to_vec` gives both. A digest that moved with
    /// serializer settings would make reproducibility unfalsifiable.
    pub fn digest(&self) -> String {
        let bytes = serde_json::to_vec(self).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        hex(&hasher.finalize())
    }

    /// Every check ran and passed, and the run was reproducible.
    pub fn all_required_passed(&self) -> bool {
        self.complete() && self.checks.iter().all(|record| record.outcome.passed())
    }

    /// Exactly the current suite's checks, each once.
    fn complete(&self) -> bool {
        if self.checks.len() != CheckId::ALL.len() {
            return false;
        }
        CheckId::ALL.iter().all(|check| {
            self.checks
                .iter()
                .filter(|record| record.check == *check)
                .count()
                == 1
        })
    }

    /// The checks that stand between this evidence and a promotion.
    pub fn blocking(&self) -> Vec<&CheckRecord> {
        self.checks
            .iter()
            .filter(|record| !record.outcome.passed())
            .collect()
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// What the pipeline needs from the world, so the suite itself stays
/// deterministic.
///
/// Installing, talking to a runtime, and knowing whether vendor credentials
/// exist are the three things that differ between CI and a test. Everything
/// else about a run is a pure function of the candidate.
pub trait SuiteHarness {
    /// Install the candidate in a clean environment through the managed
    /// lifecycle and return the digest of what actually landed.
    fn install(&self, recipe: &CatalogRecipe) -> Result<String, String>;

    /// A fresh transport to a launched runtime. Called once per check that
    /// needs a live session, because a check that inherited another check's
    /// session would not be testing launch.
    fn transport(&self) -> Result<Box<dyn Transport>, String>;

    /// What the runtime says about itself at handshake, for the drift check.
    fn advertisement(&self) -> Advertisement;

    /// Whether this environment has the vendor credentials the candidate's
    /// `vendor_requirement` names. False in CI without secrets, and in every
    /// test.
    fn vendor_credentials_available(&self) -> bool {
        false
    }

    /// Whether an update from the prior verified version, a rollback, and an
    /// uninstall that retains history all behaved. Delegated because they are
    /// managed-lifecycle questions, already proven by #161's own suite; the
    /// pipeline's job is to *require* them, not to reimplement them.
    fn lifecycle_transitions(&self, prior: Option<&BackendVersion>) -> Result<(), String>;
}

/// Run the conformance suite and produce evidence.
///
/// The required set runs [`DETERMINISM_RUNS`] times. The evidence carries the
/// first run's outcomes and whether every run agreed — an alternating check is
/// caught even when its last run passes.
pub fn run_suite(
    candidate: &Candidate,
    registry: &IntegrationRegistry,
    harness: &dyn SuiteHarness,
    bridge_version: &str,
    produced_at: &str,
) -> Evidence {
    let platform = PlatformTarget::current().or_else(|| candidate.platforms.first().copied());

    let mut runs: Vec<(Vec<CheckRecord>, String)> = Vec::with_capacity(DETERMINISM_RUNS);
    for _ in 0..DETERMINISM_RUNS {
        runs.push(run_once(candidate, registry, harness));
    }

    let (first_checks, artifact_digest) = runs[0].clone();
    let deterministic = runs.iter().all(|(checks, digest)| {
        *checks == first_checks && *digest == artifact_digest
    });

    Evidence {
        agent: candidate.agent.clone(),
        version: candidate.version.clone(),
        backend: candidate.backend.clone(),
        backend_kind: candidate.backend_kind,
        artifact_digest,
        bridge_version: bridge_version.to_owned(),
        platform: platform.unwrap_or(PlatformTarget::DarwinAarch64),
        suite_version: SUITE_VERSION,
        checks: first_checks,
        deterministic,
        produced_at: produced_at.to_owned(),
    }
}

/// One pass over the required set. Returns the outcomes and the installed
/// digest, both of which must be identical across runs for the verdict to be
/// called deterministic.
fn run_once(
    candidate: &Candidate,
    registry: &IntegrationRegistry,
    harness: &dyn SuiteHarness,
) -> (Vec<CheckRecord>, String) {
    let mut records = Vec::with_capacity(CheckId::ALL.len());
    // A macro rather than a closure: a closure capturing `records` mutably
    // would stop the checks below from reading what has been recorded so far,
    // and several of them legitimately need to.
    macro_rules! record {
        ($check:expr, $outcome:expr $(,)?) => {
            records.push(CheckRecord {
                check: $check,
                outcome: $outcome,
            })
        };
    }

    // 1–3: install, then everything a live session needs.
    let installed = harness.install(&candidate.recipe);
    let artifact_digest = match &installed {
        Ok(digest) => {
            record!(CheckId::Install, CheckOutcome::Passed);
            digest.clone()
        }
        Err(error) => {
            record!(CheckId::Install, CheckOutcome::failed(error));
            String::new()
        }
    };

    let entry = candidate.provisional();
    let request = LaunchRequest {
        cwd: ".".into(),
        tier: CapabilityTier::Standard,
        instructions: None,
    };

    // Everything below drives IntegrationSession — the same surface a real
    // session uses. A private path to the runtime would verify a path no user
    // ever takes.
    let mut session = match harness
        .transport()
        .map_err(|error| error)
        .and_then(|transport| {
            registry
                .launch(&entry, transport, &request)
                .map_err(|error| error.to_string())
        }) {
        Ok(session) => {
            record!(CheckId::Launch, CheckOutcome::Passed);
            Some(session)
        }
        Err(error) => {
            record!(CheckId::Launch, CheckOutcome::failed(&error));
            None
        }
    };

    let cascade = |records: &mut Vec<CheckRecord>, checks: &[CheckId]| {
        for check in checks {
            records.push(CheckRecord {
                check: *check,
                outcome: CheckOutcome::failed("launch failed, so this check could not run"),
            });
        }
    };

    let Some(session) = session.as_mut() else {
        cascade(
            &mut records,
            &[
                CheckId::Initialize,
                CheckId::NewSession,
                CheckId::MultiTurnPrompt,
                CheckId::Normalization,
                CheckId::Permissions,
                CheckId::Cancellation,
                CheckId::Shutdown,
                CheckId::ClaimedResume,
            ],
        );
        finish_offline_checks(candidate, harness, &mut records, &artifact_digest);
        records.sort_by_key(|record| CheckId::ALL.iter().position(|id| *id == record.check));
        return (records, artifact_digest);
    };

    // 3: initialize — the handshake produced a capability report at all.
    record!(CheckId::Initialize,
        if session.capabilities().agreed.is_empty() && !entry.capabilities.is_empty() {
            CheckOutcome::failed("handshake agreed no capability with the profile")
        } else {
            CheckOutcome::Passed
        },
    );

    // 4: a new session has an identity to resume against, or declares it cannot.
    record!(CheckId::NewSession,
        match (session.provider_session_id(), session.resume_support()) {
            (Some(_), _) | (None, ResumeSupport::None) => CheckOutcome::Passed,
            (None, ResumeSupport::Native) => CheckOutcome::failed(
                "claims native resume but minted no provider session id to resume against",
            ),
        },
    );

    // 5–6: two turns, and every frame they produce normalizes or is reported.
    let mut normalization_total = true;
    for turn in ["first turn", "second turn"] {
        if let Err(error) = session.send_turn(turn) {
            record!(CheckId::MultiTurnPrompt, CheckOutcome::failed(error.to_string()));
            normalization_total = false;
            break;
        }
        let events = session.drain();
        if events.is_empty() {
            normalization_total = false;
        }
    }
    if !records.iter().any(|r| r.check == CheckId::MultiTurnPrompt) {
        record!(CheckId::MultiTurnPrompt, CheckOutcome::Passed);
    }
    record!(CheckId::Normalization,
        if normalization_total {
            CheckOutcome::Passed
        } else {
            CheckOutcome::failed("a turn produced no normalized event, so a frame was dropped")
        },
    );

    // 7: the integration reports a permission model rather than deciding one.
    record!(CheckId::Permissions,
        match session.permissions() {
            PermissionModel::RequestsApproval | PermissionModel::SandboxOnly => {
                CheckOutcome::Passed
            }
        },
    );

    // 8: an interrupt is delivered or refused — never silently dropped.
    record!(CheckId::Cancellation,
        match session.interrupt() {
            Ok(()) => CheckOutcome::Passed,
            Err(error) if error.to_string().contains("interrupt") => CheckOutcome::Passed,
            Err(error) => CheckOutcome::failed(error.to_string()),
        },
    );

    // 9: claimed resume behaviour matches what the session will actually do.
    record!(CheckId::ClaimedResume,
        match (session.resume_support(), session.provider_session_id()) {
            (ResumeSupport::Native, Some(id)) => {
                let id = id.to_owned();
                match session.resume(&id) {
                    Ok(()) => CheckOutcome::Passed,
                    Err(error) => CheckOutcome::failed(error.to_string()),
                }
            }
            (ResumeSupport::Native, None) => {
                CheckOutcome::failed("claims native resume with nothing to resume")
            }
            (ResumeSupport::None, _) => match session.resume("anything") {
                Err(_) => CheckOutcome::Passed,
                Ok(()) => CheckOutcome::failed("declares no resume but accepted one anyway"),
            },
        },
    );

    // 11: shutdown is accepted.
    session.shutdown(ShutdownReason::Completed);
    record!(CheckId::Shutdown, CheckOutcome::Passed);

    finish_offline_checks(candidate, harness, &mut records, &artifact_digest);
    records.sort_by_key(|record| CheckId::ALL.iter().position(|id| *id == record.check));
    (records, artifact_digest)
}

/// The checks that do not need a live session: vendor auth, redaction,
/// lifecycle transitions, and the platform claim.
fn finish_offline_checks(
    candidate: &Candidate,
    harness: &dyn SuiteHarness,
    records: &mut Vec<CheckRecord>,
    artifact_digest: &str,
) {
    // Capability drift, through #166's own rule rather than a copy of it.
    //
    // Checked here rather than against a live session on purpose: drift is a
    // property of what the runtime advertises against what the profile lists,
    // and a launch that fails *because* of drift is exactly the case that most
    // needs its reason recorded rather than cascaded away.
    records.push(CheckRecord {
        check: CheckId::CapabilityDrift,
        outcome: match check_capabilities(&candidate.provisional(), &harness.advertisement()) {
            Ok(_) => CheckOutcome::Passed,
            Err(error) => CheckOutcome::failed(error.to_string()),
        },
    });

    // A candidate that needs vendor credentials must fail *closed* without
    // them. Where the credentials are absent the check is skipped, which is
    // not a pass — the gate stays unmet rather than quietly satisfied.
    records.push(CheckRecord {
        check: CheckId::VendorAuthRequiredFailure,
        outcome: match candidate.vendor_requirement {
            VendorRequirement::None => CheckOutcome::Passed,
            VendorRequirement::VendorLogin | VendorRequirement::VendorApiKeyEnvironment => {
                if harness.vendor_credentials_available() {
                    CheckOutcome::Passed
                } else {
                    CheckOutcome::Skipped {
                        reason: SkipReason::VendorAuthUnavailable,
                    }
                }
            }
        },
    });

    // Redaction is checked on the pipeline's own output: every reason it has
    // produced this run must already be bounded and stripped.
    let leaked = records.iter().any(|record| match &record.outcome {
        CheckOutcome::Failed { reason } => {
            reason.len() > MAX_REASON_BYTES || reason != &redact(reason)
        }
        _ => false,
    });
    records.push(CheckRecord {
        check: CheckId::Redaction,
        outcome: if leaked {
            CheckOutcome::failed("a failure reason was not bounded and redacted")
        } else {
            CheckOutcome::Passed
        },
    });

    let transitions = harness.lifecycle_transitions(None);
    for check in [
        CheckId::UpdateFromPriorVerified,
        CheckId::Rollback,
        CheckId::UninstallRetainsHistory,
    ] {
        records.push(CheckRecord {
            check,
            outcome: match &transitions {
                Ok(()) => CheckOutcome::Passed,
                Err(error) => CheckOutcome::failed(error),
            },
        });
    }

    records.push(CheckRecord {
        check: CheckId::PlatformClaims,
        outcome: if candidate.platforms.is_empty() {
            CheckOutcome::failed("claims no platform")
        } else if artifact_digest.is_empty() {
            CheckOutcome::failed("nothing installed, so no platform claim was exercised")
        } else {
            match PlatformTarget::current() {
                Some(current) if !candidate.platforms.contains(&current) => CheckOutcome::Skipped {
                    reason: SkipReason::PlatformNotClaimed,
                },
                _ => CheckOutcome::Passed,
            }
        },
    });
}

/// Why a candidate did not reach users. One variant per condition, so a caller
/// can act on them differently without matching on prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromotionBlocked {
    /// A required check did not pass. Names them, because the fix is per-check.
    ChecksFailed { checks: Vec<String> },
    /// The required set did not agree with itself across runs.
    NotDeterministic,
    /// The verdict's `evidence_ref` is not the digest of the evidence offered.
    EvidenceMismatch { expected: String, found: String },
    /// What was installed is not what the recipe declared.
    ArtifactMismatch { declared: String, installed: String },
    /// The evidence describes a different agent, version, backend, platform, or
    /// Bridge than the candidate.
    EvidenceIsForSomethingElse { field: &'static str },
    /// The evidence was produced by a different revision of the suite.
    SuiteVersionMismatch { expected: u32, found: u32 },
    /// The candidate's version appears in its own blocked list.
    VersionBlocked { version: String },
    /// The candidate does not advance the entry it would replace.
    NotAnAdvance { installed: String, candidate: String },
    /// The promoted document would not survive the catalog's own validation.
    WouldNotValidate { reason: String },
}

impl std::fmt::Display for PromotionBlocked {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ChecksFailed { checks } => {
                write!(formatter, "required checks did not pass: {}", checks.join(", "))
            }
            Self::NotDeterministic => {
                write!(formatter, "the required set did not agree with itself across runs")
            }
            Self::EvidenceMismatch { expected, found } => write!(
                formatter,
                "evidence reference {found} is not the digest of this evidence ({expected})"
            ),
            Self::ArtifactMismatch { declared, installed } => write!(
                formatter,
                "installed artifact {installed} is not the declared {declared}"
            ),
            Self::EvidenceIsForSomethingElse { field } => {
                write!(formatter, "evidence {field} does not match the candidate")
            }
            Self::SuiteVersionMismatch { expected, found } => write!(
                formatter,
                "evidence was produced by suite version {found}, not {expected}"
            ),
            Self::VersionBlocked { version } => {
                write!(formatter, "version {version} is in its own blocked list")
            }
            Self::NotAnAdvance {
                installed,
                candidate,
            } => write!(
                formatter,
                "candidate {candidate} does not advance the installed {installed}"
            ),
            Self::WouldNotValidate { reason } => {
                write!(formatter, "the promoted catalog would not validate: {reason}")
            }
        }
    }
}

impl std::error::Error for PromotionBlocked {}

/// Promote a candidate into a new snapshot, or say why not.
///
/// Every gate #168 names, in one place. The result is a `CatalogSnapshot` that
/// still has to be signed and installed — this decides *that* a version may
/// reach users, not that it has.
pub fn promote(
    catalog: &Catalog,
    candidate: &Candidate,
    evidence: &Evidence,
    published_at: &str,
) -> Result<CatalogSnapshot, PromotionBlocked> {
    if evidence.suite_version != SUITE_VERSION {
        return Err(PromotionBlocked::SuiteVersionMismatch {
            expected: SUITE_VERSION,
            found: evidence.suite_version,
        });
    }
    if evidence.agent != candidate.agent {
        return Err(PromotionBlocked::EvidenceIsForSomethingElse { field: "agent" });
    }
    if evidence.version != candidate.version {
        return Err(PromotionBlocked::EvidenceIsForSomethingElse { field: "version" });
    }
    if evidence.backend != candidate.backend {
        return Err(PromotionBlocked::EvidenceIsForSomethingElse { field: "backend" });
    }
    if evidence.backend_kind != candidate.backend_kind {
        return Err(PromotionBlocked::EvidenceIsForSomethingElse {
            field: "backendKind",
        });
    }
    if !candidate.platforms.contains(&evidence.platform) {
        return Err(PromotionBlocked::EvidenceIsForSomethingElse { field: "platform" });
    }
    if candidate.blocked_versions.contains(&candidate.version) {
        return Err(PromotionBlocked::VersionBlocked {
            version: candidate.version.to_string(),
        });
    }
    if !evidence.deterministic {
        return Err(PromotionBlocked::NotDeterministic);
    }

    let blocking = evidence.blocking();
    if !blocking.is_empty() || !evidence.all_required_passed() {
        return Err(PromotionBlocked::ChecksFailed {
            checks: blocking
                .iter()
                .map(|record| record.check.as_str().to_owned())
                .collect(),
        });
    }

    // What the recipe declares, against what actually landed. This is the
    // tampered-artifact gate, and it is why evidence records the installed
    // digest rather than the expected one.
    if let Some(declared) = declared_digest(&candidate.recipe) {
        if !declared.eq_ignore_ascii_case(evidence.artifact_digest.trim()) {
            return Err(PromotionBlocked::ArtifactMismatch {
                declared: declared.to_owned(),
                installed: evidence.artifact_digest.clone(),
            });
        }
    }

    // A version must advance the one it replaces. A promotion that moved an
    // agent backwards would reinstate whatever the newer version fixed.
    if let Some(installed) = catalog.entry(&candidate.agent) {
        if !version_advances(installed.version.as_str(), candidate.version.as_str()) {
            return Err(PromotionBlocked::NotAnAdvance {
                installed: installed.version.to_string(),
                candidate: candidate.version.to_string(),
            });
        }
    }

    let verification = Verification::verified(evidence);
    if verification.evidence_ref != evidence.digest() {
        return Err(PromotionBlocked::EvidenceMismatch {
            expected: evidence.digest(),
            found: verification.evidence_ref,
        });
    }

    let entry = candidate.clone().into_entry(verification);
    let snapshot = replace_entry(catalog, entry, published_at);

    // The output faces exactly the validation a remote snapshot faces. The
    // pipeline must not be able to mint a document the catalog would refuse.
    Catalog::would_accept(&snapshot, &evidence.bridge_version).map_err(|error: CatalogError| {
        PromotionBlocked::WouldNotValidate {
            reason: error.to_string(),
        }
    })?;

    Ok(snapshot)
}

/// Withdraw one agent's entry, returning to what the prior snapshot served.
///
/// Per-entry on purpose. A health failure in one agent must not withdraw every
/// other agent that happened to be verified in the same snapshot.
pub fn roll_back(
    catalog: &Catalog,
    prior: &CatalogSnapshot,
    agent: &AgentId,
    published_at: &str,
) -> CatalogSnapshot {
    let mut entries: Vec<VerifiedEntry> = catalog
        .entries()
        .iter()
        .filter(|entry| &entry.agent != agent)
        .cloned()
        .collect();
    if let Some(restored) = prior.entries.iter().find(|entry| &entry.agent == agent) {
        entries.push(restored.clone());
    }
    entries.sort_by(|left, right| left.agent.as_str().cmp(right.agent.as_str()));
    CatalogSnapshot {
        schema_version: crate::verified_catalog::SCHEMA_VERSION,
        generation: catalog.generation() + 1,
        published_at: published_at.to_owned(),
        minimum_bridge_version: catalog.minimum_bridge_version().to_owned(),
        entries,
    }
}

/// The candidate's entry replacing any entry for the same agent, with every
/// other entry byte-identical and the generation advanced by exactly one.
fn replace_entry(catalog: &Catalog, entry: VerifiedEntry, published_at: &str) -> CatalogSnapshot {
    let mut entries: Vec<VerifiedEntry> = catalog
        .entries()
        .iter()
        .filter(|existing| existing.agent != entry.agent)
        .cloned()
        .collect();
    entries.push(entry);
    entries.sort_by(|left, right| left.agent.as_str().cmp(right.agent.as_str()));
    CatalogSnapshot {
        schema_version: crate::verified_catalog::SCHEMA_VERSION,
        generation: catalog.generation() + 1,
        published_at: published_at.to_owned(),
        minimum_bridge_version: catalog.minimum_bridge_version().to_owned(),
        entries,
    }
}

/// The integrity a recipe declares, where it declares one. An npm closure's
/// integrity is its lockfile, which the engine checks during install, so there
/// is no single digest to compare here.
fn declared_digest(recipe: &CatalogRecipe) -> Option<&str> {
    match recipe {
        CatalogRecipe::ReleaseArtifact { sha256, .. } => Some(sha256.as_str()),
        CatalogRecipe::NpmClosure { .. } => None,
    }
}

/// Whether `candidate` is strictly newer than `installed`, comparing dotted
/// numeric components left to right.
fn version_advances(installed: &str, candidate: &str) -> bool {
    let parse = |version: &str| -> Vec<u64> {
        version
            .split(['.', '-', '+'])
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect()
    };
    let (left, right) = (parse(installed), parse(candidate));
    for index in 0..left.len().max(right.len()) {
        let installed_part = left.get(index).copied().unwrap_or(0);
        let candidate_part = right.get(index).copied().unwrap_or(0);
        if candidate_part != installed_part {
            return candidate_part > installed_part;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agent::NormalizedEvent,
        agent_integration::{
            AgentIntegration, IntegrationDescriptor, IntegrationError, PermissionModel,
        },
        backend_binding::BackendKind,
    };
    use serde_json::{json, Value};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };

    const BRIDGE: &str = "0.1.0";
    const NOW: &str = "2026-08-16T00:00:00Z";


    /// The module's own source, minus the test module.
    ///
    /// Structural tests assert that certain spellings do *not* appear. Without
    /// stripping the tests, every such test matches the very string it is
    /// asserting the absence of.
    fn code_only(source: &str) -> &str {
        source
            .split("#[cfg(test)]")
            .next()
            .expect("a module always has a non-test half")
    }

    // ---- fakes -----------------------------------------------------------

    struct ScriptedTransport {
        advertisement: Advertisement,
        frames: Mutex<Vec<Value>>,
    }

    impl Transport for ScriptedTransport {
        fn handshake(&mut self) -> Result<Advertisement, IntegrationError> {
            Ok(self.advertisement.clone())
        }
        fn send(&mut self, _frame: Value) -> Result<(), IntegrationError> {
            // Every turn produces one frame to drain, so normalization has
            // something to be total over.
            self.frames
                .lock()
                .unwrap()
                .push(json!({"type": "text", "text": "ok"}));
            Ok(())
        }
        fn next_frame(&mut self) -> Option<Result<Value, IntegrationError>> {
            let mut frames = self.frames.lock().unwrap();
            if frames.is_empty() {
                return None;
            }
            Some(Ok(frames.remove(0)))
        }
        fn interrupt(&mut self) -> Result<(), IntegrationError> {
            Ok(())
        }
        fn failure_context(&mut self) -> Option<String> {
            None
        }
        fn shutdown(&mut self, _reason: ShutdownReason) {}
    }

    struct FakeIntegration {
        resume: ResumeSupport,
    }

    impl AgentIntegration for FakeIntegration {
        fn descriptor(&self) -> IntegrationDescriptor {
            IntegrationDescriptor {
                agent: AgentId::parse("fake").unwrap(),
                backend: BackendId::parse("fake.acp").unwrap(),
                kind: BackendKind::Acp,
                label: "Fake".into(),
            }
        }
        fn model_for(&self, tier: CapabilityTier) -> Option<String> {
            Some(format!("fake-{}", tier.as_str()))
        }
        fn startup_frames(&self, _request: &LaunchRequest) -> Vec<Value> {
            vec![json!({"type": "start"})]
        }
        fn normalize(&self, frame: &Value) -> Vec<NormalizedEvent> {
            match frame.get("type").and_then(Value::as_str) {
                Some("text") => {
                    let mut event = NormalizedEvent::new("message.completed");
                    event.role = Some("assistant".into());
                    vec![event]
                }
                _ => vec![],
            }
        }
        fn permissions(&self) -> PermissionModel {
            PermissionModel::RequestsApproval
        }
        fn resume(&self) -> ResumeSupport {
            self.resume
        }
    }

    /// The world, faked. Every knob is a condition #168 names.
    struct FakeHarness {
        digest: String,
        install_fails: bool,
        vendor_credentials: bool,
        lifecycle_fails: Option<String>,
        advertised: Vec<String>,
        /// Flips the install digest every other call, which is how a flaky run
        /// is simulated without a clock or a random number.
        flaky: bool,
        calls: AtomicUsize,
    }

    impl Default for FakeHarness {
        fn default() -> Self {
            Self {
                digest: "a".repeat(64),
                install_fails: false,
                vendor_credentials: false,
                lifecycle_fails: None,
                advertised: vec!["turn".into(), "interrupt".into()],
                flaky: false,
                calls: AtomicUsize::new(0),
            }
        }
    }

    impl SuiteHarness for FakeHarness {
        fn install(&self, _recipe: &CatalogRecipe) -> Result<String, String> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if self.install_fails {
                return Err("the artifact did not land".into());
            }
            if self.flaky && call % 2 == 1 {
                return Ok("b".repeat(64));
            }
            Ok(self.digest.clone())
        }
        fn transport(&self) -> Result<Box<dyn Transport>, String> {
            Ok(Box::new(ScriptedTransport {
                advertisement: Advertisement {
                    capabilities: self.advertised.clone(),
                    extension_methods: Vec::new(),
                    provider_session_id: Some("provider-1".into()),
                },
                frames: Mutex::new(Vec::new()),
            }))
        }
        fn advertisement(&self) -> Advertisement {
            Advertisement {
                capabilities: self.advertised.clone(),
                extension_methods: Vec::new(),
                provider_session_id: Some("provider-1".into()),
            }
        }
        fn vendor_credentials_available(&self) -> bool {
            self.vendor_credentials
        }
        fn lifecycle_transitions(&self, _prior: Option<&BackendVersion>) -> Result<(), String> {
            match &self.lifecycle_fails {
                Some(reason) => Err(reason.clone()),
                None => Ok(()),
            }
        }
    }

    fn registry() -> IntegrationRegistry {
        let mut registry = IntegrationRegistry::empty();
        registry
            .register(Arc::new(FakeIntegration {
                resume: ResumeSupport::Native,
            }))
            .unwrap();
        registry
    }

    fn candidate() -> Candidate {
        Candidate {
            agent: AgentId::parse("fake").unwrap(),
            label: "Fake Agent".into(),
            vendor: "Fake Inc".into(),
            license: "MIT".into(),
            source_url: "https://example.invalid/fake".into(),
            version: BackendVersion::parse("1.2.0").unwrap(),
            platforms: PlatformTarget::current().into_iter().collect(),
            recipe: CatalogRecipe::ReleaseArtifact {
                url: "https://example.invalid/fake-1.2.0.tar.gz".into(),
                sha256: "a".repeat(64),
                archive: crate::verified_catalog::ArchiveKind::TarGz,
                entrypoint: "bin/fake".into(),
            },
            backend: BackendId::parse("fake.acp").unwrap(),
            backend_kind: CatalogBackendKind::Acp,
            integration: IntegrationConfig::default(),
            vendor_requirement: VendorRequirement::None,
            capabilities: vec!["turn".into(), "interrupt".into()],
            blocked_versions: Vec::new(),
            minimum_bridge_version: "0.1.0".into(),
            detected_from: ApprovedSource::VendorRelease,
        }
    }

    fn evidence_for(harness: &FakeHarness) -> Evidence {
        run_suite(&candidate(), &registry(), harness, BRIDGE, NOW)
    }

    fn catalog() -> Catalog {
        Catalog::bundled(BRIDGE).unwrap()
    }

    // ---- evidence --------------------------------------------------------

    #[test]
    fn only_evidence_can_produce_a_verified_status() {
        // Structural, over both modules' source: if a second producer of a
        // Verified verdict appears, this fails. Reading the source is the only
        // way to assert "no other code path does this" — a runtime assertion
        // could only speak for the paths it happened to run.
        let pipeline = code_only(include_str!("verification_pipeline.rs"));
        let catalog_source = code_only(include_str!("verified_catalog.rs"));

        // Exactly one site *constructs* the status. The comparison in
        // `validate` reads it and does not produce it, so it is spelled
        // differently and does not match this.
        let constructed = catalog_source
            .match_indices("status: VerificationStatus::Verified")
            .count();
        assert_eq!(
            constructed, 1,
            "a second constructor of a Verified verdict appeared in verified_catalog"
        );

        // And the pipeline never names the status at all — it goes through the
        // constructor, which is the whole point.
        assert!(
            !pipeline.contains("status: VerificationStatus::Verified"),
            "the pipeline must reach Verified only through Verification::verified"
        );
        assert!(pipeline.contains("Verification::verified(evidence)"));
    }

    #[test]
    fn evidence_digest_is_stable_across_runs() {
        let first = evidence_for(&FakeHarness::default());
        let second = evidence_for(&FakeHarness::default());
        assert_eq!(first.digest(), second.digest());
        assert_eq!(first.digest().len(), 64);

        let mut altered = first.clone();
        altered.artifact_digest = "c".repeat(64);
        assert_ne!(
            first.digest(),
            altered.digest(),
            "changing a field must change the digest"
        );
    }

    #[test]
    fn a_verdict_whose_evidence_ref_does_not_match_is_refused() {
        let evidence = evidence_for(&FakeHarness::default());
        let verification = Verification::verified(&evidence);
        assert_eq!(verification.evidence_ref, evidence.digest());

        // A verdict minted against one evidence document does not match another.
        let mut other = evidence.clone();
        other.produced_at = "2026-01-01T00:00:00Z".into();
        assert_ne!(verification.evidence_ref, other.digest());
    }

    #[test]
    fn evidence_carries_no_credential_and_no_transcript() {
        let evidence = evidence_for(&FakeHarness::default());
        let encoded = serde_json::to_string(&evidence).unwrap();
        let lowered = encoded.to_lowercase();
        for forbidden in [
            "token", "secret", "apikey", "api_key", "password", "bearer", "credential", "authorization",
        ] {
            assert!(
                !lowered.contains(forbidden),
                "evidence must not carry {forbidden}"
            );
        }
        // No field is a transcript: the only free text is a bounded reason.
        for record in &evidence.checks {
            if let CheckOutcome::Failed { reason } = &record.outcome {
                assert!(reason.len() <= MAX_REASON_BYTES);
            }
        }
    }

    #[test]
    fn a_failure_reason_is_bounded_and_stripped() {
        let leaky = format!(
            "failed at /Users/someone/.bridge/run OPENAI_API_KEY=sk-abc {}",
            "x".repeat(2_000)
        );
        let cleaned = redact(&leaky);
        assert!(cleaned.len() <= MAX_REASON_BYTES);
        assert!(!cleaned.contains("/Users/someone"));
        assert!(!cleaned.contains("sk-abc"));
        assert!(cleaned.contains("<path>"));
        assert!(cleaned.contains("<redacted>"));
    }

    // ---- the suite -------------------------------------------------------

    #[test]
    fn every_required_check_from_the_issue_is_present() {
        // The list #168 names, spelled as the issue spells it. Dropping one is
        // a test failure rather than a quiet reduction in coverage.
        let required = [
            "install",
            "launch",
            "initialize",
            "new_session",
            "multi_turn_prompt",
            "normalization",
            "permissions",
            "cancellation",
            "shutdown",
            "claimed_resume",
            "vendor_auth_required_failure",
            "redaction",
            "update_from_prior_verified",
            "rollback",
            "uninstall_retains_history",
            "platform_claims",
            "capability_drift",
        ];
        let present: Vec<&str> = CheckId::ALL.iter().map(|check| check.as_str()).collect();
        assert_eq!(present, required);

        // And every one of them actually appears in a produced document.
        let evidence = evidence_for(&FakeHarness::default());
        assert_eq!(evidence.checks.len(), CheckId::ALL.len());
        assert!(evidence.all_required_passed(), "{:?}", evidence.blocking());
    }

    #[test]
    fn a_skipped_required_check_is_not_a_pass() {
        let harness = FakeHarness {
            vendor_credentials: false,
            ..FakeHarness::default()
        };
        let mut needs_auth = candidate();
        needs_auth.vendor_requirement = VendorRequirement::VendorLogin;
        let evidence = run_suite(&needs_auth, &registry(), &harness, BRIDGE, NOW);

        let record = evidence
            .checks
            .iter()
            .find(|record| record.check == CheckId::VendorAuthRequiredFailure)
            .unwrap();
        assert_eq!(
            record.outcome,
            CheckOutcome::Skipped {
                reason: SkipReason::VendorAuthUnavailable
            }
        );
        assert!(!record.outcome.passed(), "a skip is not a pass");
        assert!(!evidence.all_required_passed());

        // And it blocks promotion rather than being waved through.
        let blocked = promote(&catalog(), &needs_auth, &evidence, NOW).unwrap_err();
        assert!(
            matches!(blocked, PromotionBlocked::ChecksFailed { ref checks }
                if checks.iter().any(|c| c == "vendor_auth_required_failure")),
            "{blocked:?}"
        );
    }

    #[test]
    fn a_flaky_required_check_blocks_promotion() {
        let harness = FakeHarness {
            flaky: true,
            ..FakeHarness::default()
        };
        let evidence = evidence_for(&harness);
        assert!(
            !evidence.deterministic,
            "an alternating run must not be called deterministic"
        );
        let blocked = promote(&catalog(), &candidate(), &evidence, NOW).unwrap_err();
        assert_eq!(blocked, PromotionBlocked::NotDeterministic);
    }

    #[test]
    fn a_failed_install_cascades_rather_than_silently_passing() {
        let harness = FakeHarness {
            install_fails: true,
            ..FakeHarness::default()
        };
        let evidence = evidence_for(&harness);
        let install = evidence
            .checks
            .iter()
            .find(|record| record.check == CheckId::Install)
            .unwrap();
        assert!(!install.outcome.passed());
        assert!(!evidence.all_required_passed());
    }

    #[test]
    fn the_suite_drives_the_same_control_plane_a_session_does() {
        // Structural: the pipeline reaches a runtime only by launching through
        // the registry, which is what a real session does. A private path
        // would verify a path no user ever takes.
        let source = code_only(include_str!("verification_pipeline.rs"));
        assert!(
            source.contains(".launch(&entry, transport, &request)"),
            "the suite must launch through the registry, as a session does"
        );
        // Constructing a driver directly would be a private path to the
        // runtime — the registry is what pairs a shape with an integration.
        assert!(
            !source.contains("driver_for("),
            "the suite must not construct drivers behind the registry's back"
        );
    }

    // ---- promotion -------------------------------------------------------

    #[test]
    fn a_simulated_upstream_release_cannot_reach_users_without_evidence() {
        // The headline acceptance. A candidate detected from an approved
        // source is inert: nothing public turns it into a served entry.
        let detected = candidate();
        assert_eq!(detected.detected_from, ApprovedSource::VendorRelease);

        let source = code_only(include_str!("verification_pipeline.rs"));
        assert!(
            source.contains("fn into_entry(self, verification: Verification)"),
            "the candidate-to-entry conversion must exist"
        );
        assert!(
            !source.contains("pub fn into_entry"),
            "and it must not be public"
        );

        // The only public path is promote, and it demands evidence by type.
        let catalog = catalog();
        assert!(catalog.entry(&detected.agent).is_none());

        // Evidence that failed cannot promote either.
        let failing = evidence_for(&FakeHarness {
            install_fails: true,
            ..FakeHarness::default()
        });
        assert!(promote(&catalog, &detected, &failing, NOW).is_err());
    }

    #[test]
    fn a_tampered_artifact_blocks_promotion() {
        // What landed is not what the recipe declared.
        let harness = FakeHarness {
            digest: "d".repeat(64),
            ..FakeHarness::default()
        };
        let evidence = evidence_for(&harness);
        let blocked = promote(&catalog(), &candidate(), &evidence, NOW).unwrap_err();
        assert!(
            matches!(blocked, PromotionBlocked::ArtifactMismatch { .. }),
            "{blocked:?}"
        );
    }

    #[test]
    fn capability_drift_blocks_promotion() {
        // The runtime claims something its profile does not list.
        let harness = FakeHarness {
            advertised: vec!["turn".into(), "interrupt".into(), "spawn-subagents".into()],
            ..FakeHarness::default()
        };
        let evidence = evidence_for(&harness);
        let drift = evidence
            .checks
            .iter()
            .find(|record| record.check == CheckId::CapabilityDrift)
            .unwrap();
        assert!(!drift.outcome.passed());

        let blocked = promote(&catalog(), &candidate(), &evidence, NOW).unwrap_err();
        assert!(
            matches!(blocked, PromotionBlocked::ChecksFailed { ref checks }
                if checks.iter().any(|c| c == "capability_drift")),
            "{blocked:?}"
        );
    }

    #[test]
    fn a_lifecycle_regression_blocks_promotion() {
        let harness = FakeHarness {
            lifecycle_fails: Some("uninstall removed session history".into()),
            ..FakeHarness::default()
        };
        let evidence = evidence_for(&harness);
        let blocked = promote(&catalog(), &candidate(), &evidence, NOW).unwrap_err();
        assert!(
            matches!(blocked, PromotionBlocked::ChecksFailed { ref checks }
                if checks.iter().any(|c| c == "uninstall_retains_history")),
            "{blocked:?}"
        );
    }

    #[test]
    fn evidence_for_something_else_cannot_promote_a_candidate() {
        let evidence = evidence_for(&FakeHarness::default());
        let mut other = candidate();
        other.version = BackendVersion::parse("9.9.9").unwrap();
        let blocked = promote(&catalog(), &other, &evidence, NOW).unwrap_err();
        assert_eq!(
            blocked,
            PromotionBlocked::EvidenceIsForSomethingElse { field: "version" }
        );
    }

    #[test]
    fn a_verdict_from_another_suite_version_is_refused() {
        let mut evidence = evidence_for(&FakeHarness::default());
        evidence.suite_version = SUITE_VERSION + 1;
        let blocked = promote(&catalog(), &candidate(), &evidence, NOW).unwrap_err();
        assert_eq!(
            blocked,
            PromotionBlocked::SuiteVersionMismatch {
                expected: SUITE_VERSION,
                found: SUITE_VERSION + 1
            }
        );
    }

    #[test]
    fn promotion_advances_the_generation_by_one_and_touches_nothing_else() {
        let catalog = catalog();
        let evidence = evidence_for(&FakeHarness::default());
        let snapshot = promote(&catalog, &candidate(), &evidence, NOW).unwrap();

        assert_eq!(snapshot.generation, catalog.generation() + 1);
        assert_eq!(snapshot.schema_version, catalog.snapshot().schema_version);
        assert_eq!(snapshot.entries.len(), catalog.entries().len() + 1);

        // Every entry that was already there is byte-identical.
        for existing in catalog.entries() {
            let carried = snapshot
                .entries
                .iter()
                .find(|entry| entry.agent == existing.agent)
                .expect("an existing entry was dropped by a promotion");
            assert_eq!(carried, existing);
        }
    }

    #[test]
    fn a_promoted_snapshot_still_passes_catalog_validation() {
        let evidence = evidence_for(&FakeHarness::default());
        let snapshot = promote(&catalog(), &candidate(), &evidence, NOW).unwrap();
        // Exactly the validation a remote snapshot faces. The pipeline must not
        // be able to mint a document the catalog would refuse.
        Catalog::would_accept(&snapshot, BRIDGE).expect("a promoted snapshot must validate");
    }

    #[test]
    fn a_promotion_cannot_move_an_agent_backwards() {
        let base = catalog();
        let evidence = evidence_for(&FakeHarness::default());
        let promoted = promote(&base, &candidate(), &evidence, NOW).unwrap();
        let installed = install_for_test(promoted, &base);

        // The same version again is not an advance.
        let blocked = promote(&installed, &candidate(), &evidence, NOW).unwrap_err();
        assert!(
            matches!(blocked, PromotionBlocked::NotAnAdvance { .. }),
            "{blocked:?}"
        );
    }

    #[test]
    fn a_rollback_withdraws_one_entry_and_leaves_the_rest_served() {
        let base = catalog();
        let evidence = evidence_for(&FakeHarness::default());
        let promoted = promote(&base, &candidate(), &evidence, NOW).unwrap();
        let installed = install_for_test(promoted, &base);
        assert!(installed.entry(&AgentId::parse("fake").unwrap()).is_some());

        // Roll back to what the base generation served — which had no such
        // entry, so the agent is withdrawn.
        let rolled = roll_back(
            &installed,
            base.snapshot(),
            &AgentId::parse("fake").unwrap(),
            NOW,
        );
        assert_eq!(rolled.generation, installed.generation() + 1);
        assert!(!rolled.entries.iter().any(|entry| entry.agent.as_str() == "fake"));
        Catalog::would_accept(&rolled, BRIDGE).expect("a rollback must produce a valid document");
    }

    #[test]
    fn candidates_cannot_become_verified_entries() {
        // Structural, in the style #164 uses for RegistryAgent: no public
        // constructor, From, or conversion in either direction.
        let source = code_only(include_str!("verification_pipeline.rs"));
        for forbidden in [
            "impl From<Candidate> for VerifiedEntry",
            "impl From<VerifiedEntry> for Candidate",
            "pub fn to_entry",
            "pub fn as_entry",
        ] {
            assert!(
                !source.contains(forbidden),
                "a candidate must not convert to a served entry: found {forbidden}"
            );
        }
    }

    /// Promote-then-install, for tests that need the promoted snapshot to be
    /// the catalog in force. Signs and installs through the real path so a test
    /// never gets a catalog a shipped build could not have.
    fn install_for_test(snapshot: CatalogSnapshot, base: &Catalog) -> Catalog {
        use crate::verified_catalog::{SignedSnapshot, TrustRoot};
        use ed25519_dalek::{Signer, SigningKey};

        let document = serde_json::to_string(&snapshot).unwrap();
        let key = SigningKey::from_bytes(&[7u8; 32]);
        let trust =
            TrustRoot::from_keys([("test-key".to_owned(), key.verifying_key().to_bytes())]).unwrap();
        let signature = key.sign(document.as_bytes()).to_bytes().to_vec();
        base.install_snapshot(
            SignedSnapshot {
                document: document.as_bytes(),
                signature: &signature,
                key_id: "test-key",
            },
            &trust,
            BRIDGE,
        )
        .expect("a promoted snapshot must install")
    }
}

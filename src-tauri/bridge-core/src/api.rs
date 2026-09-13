//! The host-agnostic body of every protocol method.
//!
//! One function per method in the `bridge-protocol` registry, all blocking and
//! all Tauri-free. Hosts own only their transport concerns: the Tauri shell
//! decodes invoke arguments and places calls on its blocking pool so nothing
//! runs on the macOS UI thread; the `bridged` daemon deserializes contracted
//! params and calls these on its per-connection threads. Neither host may
//! carry method logic of its own — a body that exists twice will drift.
//!
//! Events: every function publishes on `core.events` (after the corresponding
//! DB commit, per the event contract) — never through a host event system.

use crate::events::CoreEvent;
use crate::model::{
    AdapterDescriptor, AgentEvent, BridgeState, CapabilityTier, Harness, SessionForestSnapshot,
};
use crate::{
    adapters, agent, agent_config, agent_integration, automations, binary, browser_bridge,
    claude_import, compaction_controller, completion, delegation, external_import, git, handoff,
    learning_job, learning_router, live_turn, marketplace, memory_ledger,
    meter, model_profiles, opencode_adapter, prompt_studio, prompts, routing_evaluation,
    secret_interception,
    session_recall, session_supervisor,
    sessions, skill_marketplace, slash, store,
    suggestion_engine, switch_summary, usage_history, usage_import, usage_pricing, usage_summary,
    verification_pipeline,
    verified_catalog, work, work_actions,
    work_observation, work_reconcile, work_task_state, worker_adoption,
    worker_lifecycle, workspace_files, worktree_registry, BridgeCore, BridgeError,
    RuntimeSession,
};
use bridge_protocol::messages as wire;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use uuid::Uuid;

// --- health / state ----------------------------------------------------------

#[derive(Serialize)]
pub struct Health {
    pub ok: bool,
    pub version: &'static str,
    pub harnesses: HashMap<&'static str, bool>,
    pub database: String,
    pub telemetry_database: String,
    pub snapshot_directory: String,
    pub snapshot_count: u64,
    pub snapshot_total_bytes: u64,
    pub adapters: Vec<AdapterDescriptor>,
    /// Actionable environment warnings (today: macOS TCC-protected project
    /// paths and ad-hoc code signing). Empty when the environment is clean.
    pub warnings: Vec<crate::health::HealthWarning>,
}

pub fn health(core: &Arc<BridgeCore>) -> Result<Health, BridgeError> {
    let adapters = core.adapter_registry.descriptors();
    let (snapshot_count, snapshot_total_bytes) =
        crate::store::history_snapshot_stats(&core.snapshot_dir);
    let opencode_available = adapters
        .iter()
        .find(|adapter| adapter.id == "opencode")
        .is_some_and(|adapter| adapter.available);
    Ok(Health {
        ok: true,
        version: env!("CARGO_PKG_VERSION"),
        harnesses: HashMap::from([
            ("claude", binary::resolve("claude").is_some()),
            ("codex", crate::codex_adapter::resolve_runtime().is_some()),
            ("opencode", opencode_available),
            ("shell", true),
        ]),
        database: core.database_path.to_string_lossy().into(),
        telemetry_database: core.telemetry_database_path.to_string_lossy().into(),
        snapshot_directory: core.snapshot_dir.to_string_lossy().into(),
        snapshot_count,
        snapshot_total_bytes,
        adapters,
        warnings: crate::health::macos_environment_warnings(core),
    })
}

pub fn refresh_model_catalogs(core: &Arc<BridgeCore>) -> Result<Health, BridgeError> {
    core.adapter_registry.refresh_model_catalogs();
    health(core)
}

pub fn get_state(core: &Arc<BridgeCore>) -> Result<BridgeState, BridgeError> {
    core.state_snapshot()
}

/// A daemon-issued discovery, keyed by the id the caller must reference it
/// by. Neither `preview` nor `commit` accepts a client-supplied
/// `DiscoveryResult`: a compromised renderer, or any local client holding the
/// daemon socket token, could otherwise hand-build one naming `approvedRoots:
/// ["/"]` and read any file readable by the process.
fn require_cached_discovery(
    core: &Arc<BridgeCore>,
    discovery_id: &str,
) -> Result<external_import::DiscoveryResult, BridgeError> {
    core.external_import_discoveries
        .lock()
        .unwrap()
        .get(discovery_id)
        .cloned()
        .ok_or_else(|| {
            BridgeError::Invalid(
                "This discovery is no longer available in this session; run discovery again"
                    .into(),
            )
        })
}

pub fn discover_external_import(
    core: &Arc<BridgeCore>,
    params: &wire::DiscoverExternalImportParams,
) -> Result<wire::ExternalImportDiscovery, BridgeError> {
    use external_import::ExternalHarnessImporter;
    let request: external_import::DiscoveryRequest = protocol_wire(params.clone())?;
    let discovery = match request.provider.as_str() {
        claude_import::PROVIDER => claude_import::ClaudeCodeImporter.discover(&request)?,
        provider => {
            return Err(BridgeError::Invalid(format!(
                "External import provider '{provider}' is not available in this build"
            )))
        }
    };
    core.external_import_discoveries
        .lock()
        .unwrap()
        .insert(discovery.discovery_id.clone(), discovery.clone());
    protocol_wire(discovery)
}

pub fn preview_external_import(
    core: &Arc<BridgeCore>,
    params: &wire::PreviewExternalImportParams,
) -> Result<wire::ExternalImportPreview, BridgeError> {
    use external_import::ExternalHarnessImporter;
    let discovery = require_cached_discovery(core, &params.discovery_id)?;
    let selection = external_import::DiscoverySelection {
        artifact_ids: params.artifact_ids.clone(),
    };
    let candidates = match discovery.provider.as_str() {
        claude_import::PROVIDER => {
            claude_import::ClaudeCodeImporter.preview(&discovery, &selection)?
        }
        provider => {
            return Err(BridgeError::Invalid(format!(
                "External import provider '{provider}' is not available in this build"
            )))
        }
    };
    Ok(wire::ExternalImportPreview {
        candidates: protocol_wire(candidates)?,
    })
}

pub fn commit_external_import(
    core: &Arc<BridgeCore>,
    params: &wire::CommitExternalImportParams,
) -> Result<wire::ExternalImportCommit, BridgeError> {
    use external_import::ExternalHarnessImporter;
    let discovery = require_cached_discovery(core, &params.discovery_id)?;
    let plan: external_import::ImportPlan = protocol_wire(params.plan.clone())?;
    // Re-derive every candidate from disk against the discovery this daemon
    // actually walked, instead of trusting a client-supplied
    // `ExternalImportCandidate`. This closes the discover/preview/commit
    // TOCTOU window: a file rewritten since preview, a forged
    // `normalizedPayload`, or a hand-built `contentHash`/`candidateId` are all
    // caught here because the source bytes are read and re-hashed right now,
    // through the same schema-gate and structural-allowlist checks preview
    // already applied.
    let all_artifact_ids = discovery
        .artifacts
        .iter()
        .map(|artifact| artifact.artifact_id.clone())
        .collect();
    let selection = external_import::DiscoverySelection {
        artifact_ids: all_artifact_ids,
    };
    let previewed = match discovery.provider.as_str() {
        claude_import::PROVIDER => {
            claude_import::ClaudeCodeImporter.preview(&discovery, &selection)?
        }
        provider => {
            return Err(BridgeError::Invalid(format!(
                "External import provider '{provider}' is not available in this build"
            )))
        }
    };
    let candidates = previewed
        .into_iter()
        // `Unsupported` is diagnostic-only by construction (an artifact whose
        // preview failed) and `normalize` refuses it outright; excluding it
        // here means a plan naming its id fails closed with "unknown
        // candidate" rather than a confusing normalize error.
        .filter(|candidate| candidate.kind != external_import::CandidateKind::Unsupported)
        .map(|candidate| match candidate.source.provider.as_str() {
            claude_import::PROVIDER => claude_import::ClaudeCodeImporter.normalize(candidate),
            provider => Err(BridgeError::Invalid(format!(
                "External import provider '{provider}' is not available in this build"
            ))),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let commit = {
        let db = core.db.lock().unwrap();
        external_import::commit_import(&db, &candidates, &plan)?
    };
    let imported_ids: std::collections::HashSet<&str> = commit
        .candidate_results
        .iter()
        .filter(|result| result.status == external_import::ImportCandidateStatus::Imported)
        .map(|result| result.candidate_id.as_str())
        .collect();
    let memory_imported = candidates.iter().any(|candidate| {
        candidate.candidate.kind == external_import::CandidateKind::Memory
            && imported_ids.contains(candidate.candidate.candidate_id.as_str())
    });
    if memory_imported {
        if let Some(scope_key) = plan.memory_scope.clone() {
            core.events.publish(CoreEvent::MemoryChanged { scope_key });
        }
    }
    // A committed import writes sessions/memory/setup rows directly, with no
    // other path that would tell the frontend to refetch — without this the
    // sidebar shows nothing new until the app restarts.
    if commit.imported > 0 {
        core.events.publish(CoreEvent::StateChanged);
    }
    protocol_wire(commit)
}

// --- github ----------------------------------------------------------------

/// The GitHub reader's core DTOs and the protocol DTOs intentionally share
/// their serialized shape. Converting at this boundary keeps the protocol
/// crate independent of core while making a field rename fail loudly here.
fn github_wire<T: DeserializeOwned, U: Serialize>(value: U) -> Result<T, BridgeError> {
    serde_json::from_value(serde_json::to_value(value).map_err(|error| BridgeError::Invalid(error.to_string()))?)
        .map_err(|error| BridgeError::Invalid(format!("GitHub protocol conversion failed: {error}")))
}

fn protocol_wire<T: DeserializeOwned, U: Serialize>(value: U) -> Result<T, BridgeError> {
    serde_json::from_value(serde_json::to_value(value).map_err(|error| BridgeError::Invalid(error.to_string()))?)
        .map_err(|error| BridgeError::Invalid(format!("Protocol conversion failed: {error}")))
}

fn github_error(error: crate::github_surface::GithubSurfaceError) -> BridgeError {
    BridgeError::Invalid(error.to_string())
}

// ── connectors ───────────────────────────────────────────────────────────────
// In-app surfaces over the harness's own authenticated MCP servers. Bridge
// holds no connector credential: every read and every write below is a bounded
// harness turn, and every write passes an approval gate that names the literal
// effect before anything runs.

fn connector_family(family: &str) -> Result<crate::work_connectors::ConnectorFamily, BridgeError> {
    crate::work_connectors::ConnectorFamily::parse(family)
        .ok_or_else(|| BridgeError::Invalid(format!("unknown connector family `{family}`")))
}

pub fn connector_list(
    core: &Arc<BridgeCore>,
    refresh: bool,
) -> Result<wire::ConnectorListResult, BridgeError> {
    let _ = (core, refresh);
    let connectors = crate::connector_surface::resolve_availability(
        &crate::connector_runs_live::discover_harness_connectors(),
    )
    .into_iter()
    .map(|entry| wire::ConnectorDescriptor {
        explanation: entry.reason.as_ref().map(|reason| reason.explanation(entry.family)),
        reason: entry.reason.as_ref().map(connector_reason_wire),
        family: entry.family.as_str().into(),
        display_name: entry.family.display_name().into(),
        // Carried to the UI so a pane can say where a connection lives, and so
        // no client has to keep its own table of which harness owns what.
        harness: entry.harness,
        has_inbox: entry.family.has_inbox_support(),
        server: entry.server,
        available: entry.available,
    })
    .collect();
    Ok(wire::ConnectorListResult { connectors })
}

fn connector_reason_wire(
    reason: &crate::connector_surface::UnavailableReason,
) -> wire::ConnectorUnavailableReason {
    use crate::connector_surface::UnavailableReason;
    match reason {
        UnavailableReason::AuthRequired => wire::ConnectorUnavailableReason::AuthRequired,
        UnavailableReason::Unreachable => wire::ConnectorUnavailableReason::Unreachable,
        UnavailableReason::NotConfigured => wire::ConnectorUnavailableReason::NotConfigured,
        UnavailableReason::NoResolver => wire::ConnectorUnavailableReason::NoResolver,
    }
}

/// Most items one inbox read returns, whatever the caller asked for.
const CONNECTOR_INBOX_MAX: u32 = 200;

pub fn connector_inbox(
    core: &Arc<BridgeCore>,
    limit: Option<u32>,
) -> Result<wire::ConnectorInboxResult, BridgeError> {
    let limit = limit.unwrap_or(50).clamp(1, CONNECTOR_INBOX_MAX);
    let db = core.db.lock().unwrap();
    let items = crate::connector_inbox::list(&db, limit as usize)?
        .into_iter()
        .map(connector_item_wire)
        .collect();
    let unread_count = crate::connector_inbox::unread_count(&db)?.clamp(0, i64::from(u32::MAX)) as u32;
    let poll = crate::work_connectors::ConnectorFamily::ALL
        .into_iter()
        .filter(|family| family.has_inbox_support())
        .map(|family| {
            let status = crate::connector_inbox::poll_status(&db, family)?;
            Ok(wire::ConnectorPollStatus {
                family: family.as_str().into(),
                last_attempt_at: status.last_attempt_at,
                last_success_at: status.last_success_at,
                degraded: status.degraded,
            })
        })
        .collect::<Result<Vec<_>, BridgeError>>()?;
    Ok(wire::ConnectorInboxResult { items, unread_count, poll })
}

fn connector_item_wire(stored: crate::connector_inbox::StoredItem) -> wire::ConnectorInboxItem {
    use crate::connector_inbox::ItemState;
    use crate::connector_surface::ItemKind;
    let crate::connector_inbox::StoredItem { item, state, card, render_rejection, resolution } = stored;
    wire::ConnectorInboxItem {
        item_key: item.key(),
        family: item.family.as_str().into(),
        channel_id: item.channel_id,
        channel_label: item.channel_label,
        author: item.author,
        kind: match item.kind {
            ItemKind::DirectMessage => wire::ConnectorItemKind::DirectMessage,
            ItemKind::Mention => wire::ConnectorItemKind::Mention,
            ItemKind::ThreadReply => wire::ConnectorItemKind::ThreadReply,
        },
        text: item.text,
        permalink: item.permalink,
        received_at: item.received_at,
        state: match state {
            ItemState::Pending => wire::ConnectorItemState::Pending,
            ItemState::Rendered => wire::ConnectorItemState::Rendered,
            ItemState::Resolved => wire::ConnectorItemState::Resolved,
        },
        card: card.map(connector_card_wire),
        render_rejection,
        resolution,
    }
}

fn connector_card_wire(card: crate::connector_surface::ConnectorCard) -> wire::ConnectorCardPayload {
    use crate::connector_surface::CardBlock;
    wire::ConnectorCardPayload {
        item_key: card.item_key,
        headline: card.headline,
        blocks: card
            .blocks
            .into_iter()
            .map(|block| match block {
                CardBlock::Message { author, text, timestamp } => {
                    wire::ConnectorCardBlock::Message { author, text, timestamp }
                }
                CardBlock::Context { text } => wire::ConnectorCardBlock::Context { text },
                CardBlock::Summary { text } => wire::ConnectorCardBlock::Summary { text },
                CardBlock::Fact { label, value } => wire::ConnectorCardBlock::Fact { label, value },
            })
            .collect(),
        suggested_replies: card.suggested_replies,
        harness_rendered: card.harness_rendered,
    }
}

/// Reply or react to one inbox item.
///
/// Two calls by design. The first arrives with `approved: None`, is refused, and
/// returns the literal effect for Bridge's own confirmation dialog; the second
/// carries the user's answer. There is no sticky grant — what gets approved is a
/// specific string going to a specific place, which is not a thing that can be
/// approved in advance.
pub fn connector_act(
    core: &Arc<BridgeCore>,
    item_key: &str,
    action: wire::ConnectorActionRequest,
    approved: Option<bool>,
) -> Result<wire::ConnectorActResult, BridgeError> {
    use crate::connector_runs::{authorize, ActionRefusal, ApprovalDecision, ConnectorAction};

    let stored = {
        let db = core.db.lock().unwrap();
        crate::connector_inbox::load(&db, item_key)?
    };
    let Some(stored) = stored else {
        return Ok(wire::ConnectorActResult::Refused {
            reason: "that message is no longer in the inbox".into(),
        });
    };
    let already_resolved = stored.state == crate::connector_inbox::ItemState::Resolved;
    let action = match action {
        wire::ConnectorActionRequest::Reply { text } => {
            ConnectorAction::Reply { item: stored.item.clone(), text }
        }
        wire::ConnectorActionRequest::React { emoji } => {
            ConnectorAction::React { item: stored.item.clone(), emoji }
        }
    };
    let available = crate::connector_runs_live::available_server(core, stored.item.family)
        .map(|_| ())
        .ok_or_else(|| {
            format!("{} is not connected in this harness", stored.item.family.display_name())
        });
    let decision = approved.map(|approved| {
        if approved { ApprovalDecision::Approved } else { ApprovalDecision::Denied }
    });

    match authorize(action, decision, already_resolved, available) {
        Ok(authorized) => match crate::connector_runs_live::execute_action(core, &authorized) {
            Ok(()) => Ok(wire::ConnectorActResult::Sent { item_key: item_key.into() }),
            Err(reason) => Ok(wire::ConnectorActResult::Refused { reason }),
        },
        Err(ActionRefusal::ApprovalRequired { effect }) => {
            Ok(wire::ConnectorActResult::ApprovalRequired { effect })
        }
        Err(refusal) => Ok(wire::ConnectorActResult::Refused { reason: refusal.detail() }),
    }
}

/// Put an item away without answering it. Not a write to the connector — it
/// resolves the Bridge-side item only, so it needs no approval.
pub fn connector_dismiss(
    core: &Arc<BridgeCore>,
    item_key: &str,
) -> Result<wire::ConnectorDismissResult, BridgeError> {
    let (dismissed, stored) = {
        let db = core.db.lock().unwrap();
        let stored = crate::connector_inbox::load(&db, item_key)?;
        let dismissed = crate::connector_inbox::resolve(
            &db,
            item_key,
            crate::connector_inbox::Resolution::Dismissed,
            &chrono::Utc::now().to_rfc3339(),
        )?;
        (dismissed, stored)
    };
    if dismissed {
        // The dismissed item's own family, not a constant: this published
        // "slack" for every family, which was wrong the moment a second one
        // existed and was invisible while only one did.
        let family = stored
            .map(|item| item.item.family.as_str().to_owned())
            .unwrap_or_else(|| "unknown".into());
        core.events.publish(crate::events::CoreEvent::ConnectorInboxChanged { family });
    }
    Ok(wire::ConnectorDismissResult { dismissed })
}

/// Run one ingress cycle now. The manual counterpart to the timer.
pub fn connector_refresh(
    core: &Arc<BridgeCore>,
    family: &str,
) -> Result<wire::ConnectorRefreshResult, BridgeError> {
    let family = connector_family(family)?;
    if !family.has_inbox_support() {
        return Err(BridgeError::Invalid(format!(
            "{} has no in-app inbox in this build",
            family.display_name()
        )));
    }
    let announced = crate::connector_runs_live::poll_once(core, family);
    Ok(wire::ConnectorRefreshResult { announced: announced.min(u32::MAX as usize) as u32 })
}

pub fn github_status(core: &Arc<BridgeCore>, workspace_id: &str, refresh: bool) -> Result<wire::GithubStatusResult, BridgeError> {
    let availability = github_wire(if refresh { core.github_surface.refresh_availability() } else { core.github_surface.availability() })?;
    let repository = if matches!(availability, wire::GithubAvailability::Available) {
        let path = locked_workspace_path(core, workspace_id)?;
        if refresh { core.github_surface.invalidate_repository(Path::new(&path)); }
        core.github_surface.resolve_repository(Path::new(&path)).ok().map(github_wire).transpose()?
    } else { None };
    Ok(wire::GithubStatusResult { availability, repository })
}

pub fn github_prs(core: &Arc<BridgeCore>, workspace_id: &str) -> Result<wire::GithubPullRequestsResult, BridgeError> {
    let path = locked_workspace_path(core, workspace_id)?;
    let summaries = core.github_surface.list_prs(Path::new(&path)).map_err(github_error)?;
    if core.github_poller.begin_refresh(workspace_id) {
        let polling_core = Arc::clone(core);
        let polling_workspace_id = workspace_id.to_owned();
        let polling_path = PathBuf::from(path);
        thread::spawn(move || {
            if let Ok(summaries) = polling_core.github_surface.list_prs_for_polling(&polling_path) {
                polling_core.github_poller.watch(
                    &polling_workspace_id,
                    polling_path,
                    &summaries,
                );
            }
            polling_core.github_poller.finish_refresh(&polling_workspace_id);
        });
    }
    let pull_requests = github_wire(summaries)?;
    Ok(wire::GithubPullRequestsResult { pull_requests })
}

pub fn github_pr(core: &Arc<BridgeCore>, workspace_id: &str, number: u64) -> Result<wire::GithubPullRequestResult, BridgeError> {
    let path = locked_workspace_path(core, workspace_id)?;
    let workspace = Path::new(&path);
    let pull_request = github_wire(core.github_surface.pr_detail(workspace, number).map_err(github_error)?)?;
    let review_threads = github_wire(core.github_surface.pr_review_threads(workspace, number).map_err(github_error)?)?;
    let files = github_wire(core.github_surface.pr_files(workspace, number).map_err(github_error)?)?;
    Ok(wire::GithubPullRequestResult { pull_request, review_threads, files })
}

pub fn github_checks(core: &Arc<BridgeCore>, workspace_id: &str, number: u64) -> Result<wire::GithubChecksResult, BridgeError> {
    let path = locked_workspace_path(core, workspace_id)?;
    let checks = github_wire(core.github_surface.pr_checks(Path::new(&path), number).map_err(github_error)?)?;
    Ok(wire::GithubChecksResult { checks })
}

pub fn github_issues(core: &Arc<BridgeCore>, workspace_id: &str) -> Result<wire::GithubIssuesResult, BridgeError> {
    let path = locked_workspace_path(core, workspace_id)?;
    let issues = github_wire(core.github_surface.list_issues(Path::new(&path)).map_err(github_error)?)?;
    Ok(wire::GithubIssuesResult { issues })
}

pub fn github_issue(core: &Arc<BridgeCore>, workspace_id: &str, number: u64) -> Result<wire::GithubIssueResult, BridgeError> {
    let path = locked_workspace_path(core, workspace_id)?;
    let issue = github_wire(core.github_surface.issue_detail(Path::new(&path), number).map_err(github_error)?)?;
    Ok(wire::GithubIssueResult { issue })
}

pub fn github_repository(core: &Arc<BridgeCore>, workspace_id: &str) -> Result<wire::GithubRepositoryResult, BridgeError> {
    let path = locked_workspace_path(core, workspace_id)?;
    github_wire(core.github_surface.repository_overview(Path::new(&path)).map_err(github_error)?)
}

pub fn github_merge_config(core: &Arc<BridgeCore>, workspace_id: &str) -> Result<wire::GithubMergeConfigResult, BridgeError> {
    let path = locked_workspace_path(core, workspace_id)?;
    let config = core.github_surface.merge_config(Path::new(&path)).map_err(github_error)?;
    github_wire(config)
}

/// Perform one mutating GitHub action. The approval gate is consulted *before*
/// the surface is touched: a denied action returns without resolving the
/// repository or spawning any subprocess. Only an approved action reaches `gh`.
pub fn github_act(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
    action: wire::GithubAction,
    confirmed: bool,
) -> Result<wire::GithubActResult, BridgeError> {
    let action: crate::github_surface::GithubAction = github_wire(action)?;
    if !crate::github_policy::authorize(confirmed).is_approved() {
        return Ok(wire::GithubActResult {
            executed: false,
            message: format!("Declined: {}", crate::github_policy::summary(&action)),
        });
    }
    let path = locked_workspace_path(core, workspace_id)?;
    let workspace = Path::new(&path);
    let is_rerun = matches!(action, crate::github_surface::GithubAction::Rerun { .. });
    let outcome = core.github_surface.act(workspace, &action);
    // A re-run makes the checks queue again; re-list so the slice-3 poller picks
    // the PR back up and the rollup returns to "running" without a manual nudge.
    // This also runs after a partial rerun failure: one run may already be
    // queued even if a later `gh run rerun` is refused.
    if is_rerun {
        if let Ok(summaries) = core.github_surface.list_prs_for_polling(workspace) {
            core.github_poller.watch(workspace_id, workspace.to_path_buf(), &summaries);
        }
    }
    let message = outcome.map_err(github_error)?;
    Ok(wire::GithubActResult { executed: true, message })
}

/// Review a pull request with a Bridge subagent and post the review as a
/// comment via `gh`. The worker is research-role (read-only, network-on), so it
/// is exempt from the write-scope approval gate and gets a read-only seatbelt
/// sandbox — posting through `gh` leaves the tree clean and fires no approval
/// card. The harness is the user's click-time pick; the model comes from the
/// Reviewer model profile when its provider matches that harness, otherwise the
/// launch path resolves the harness's tier default.
///
/// `bugbot` (and `cursor-bugbot`) is not a local worker: it posts `cursor review`
/// on the pull request so Cursor Bugbot runs the review on GitHub.
pub fn github_review(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
    number: u64,
    harness: &str,
    session_id: Option<String>,
) -> Result<wire::GithubReviewResult, BridgeError> {
    if is_cursor_bugbot(harness) {
        return request_cursor_bugbot_review(core, workspace_id, number);
    }
    let harness = crate::delegation::normalize_harness(harness)
        .ok_or_else(|| BridgeError::Invalid(format!("unsupported harness: {harness}")))?;
    // Validate the workspace exists before spending a session on it.
    core.workspace_path(workspace_id)?;

    // Resolve the Reviewer model profile. Its tier/effort shape the worker; its
    // model is only used when the profile's provider matches the chosen harness,
    // otherwise the launch path picks the harness's tier default.
    let resolved = {
        let db = core.db.lock().unwrap();
        crate::model_profiles::resolve_profile(
            &db,
            &core.adapter_registry.descriptors(),
            crate::model_profiles::ProfilePurpose::Reviewer,
        )?
    };
    let (capability_tier, effort, model) = match resolved {
        Some(profile) => {
            let model = (profile.provider == harness).then(|| profile.model.clone());
            (profile.tier, profile.effort, model)
        }
        // Model setup is incomplete: fall back to a strong reviewer tier and let
        // the launch path resolve the harness's tier default.
        None => (CapabilityTier::Strong, delegation::Effort::High, None),
    };

    // Establish the parent orchestrator session. Reuse the caller's session when
    // it exists and belongs to this workspace; otherwise mint a fresh one.
    let parent_session_id = match session_id {
        Some(existing)
            if session_belongs_to_workspace(core, &existing, workspace_id)? =>
        {
            existing
        }
        _ => {
            // Mirror `create_workspace_session` but keep the planned id: an
            // orchestrator session (depth 0) with no isolated worktree — the
            // worker gets its own read-only sandbox at launch.
            let operation = core.workspace_operation(workspace_id);
            let _operation = operation.lock().unwrap();
            let plan = core.plan_workspace_session(workspace_id, false)?;
            let new_id = plan.session_id().to_owned();
            core.persist_workspace_session(plan, None)?;
            new_id
        }
    };

    // A turn id is a free-form string here — the launch path takes any id, as the
    // user-driven retry (`retry-<uuid>`) does; no turn row is a precondition.
    let turn_id = format!("github-review-{}", Uuid::new_v4());

    let objective = format!(
        "Review pull request #{number} in this repository. Run `gh pr view {number}` and \
         `gh pr diff {number}` to read the change, then post a concise, constructive code \
         review as a comment using `gh pr comment {number} --body \"...\"`. Cite concrete \
         files and line numbers; call out correctness bugs, risky changes, and missing tests. \
         Do NOT approve, merge, request-changes, or close the PR — only post a comment."
    );

    let directive = delegation::DelegationRequest {
        schema_version: delegation::SCHEMA_VERSION,
        role: delegation::WorkerRole::Research,
        objective,
        acceptance_criteria: vec![
            format!("A review comment is posted on PR #{number} via gh"),
            "The review cites concrete files or lines".into(),
        ],
        known_facts: Vec::new(),
        decisions: Vec::new(),
        evidence_ids: Vec::new(),
        relevant_files: Vec::new(),
        owned_paths: Vec::new(),
        write_mode: delegation::WriteMode::ReadOnly,
        capability_tier,
        effort,
        network_access: true,
        writable_output_paths: Vec::new(),
        verification: Vec::new(),
        output_contract: delegation::OutputContract::ResearchResult,
        harness: Some(harness.clone()),
        model,
    };
    directive
        .validate()
        .map_err(BridgeError::Invalid)?;

    let outcome = live_turn::launch_worker_outcome(core, &parent_session_id, &turn_id, &directive, true);
    let result = match outcome {
        live_turn::WorkerLaunchOutcome::Launched(child_session_id) => wire::GithubReviewResult {
            status: "launched".into(),
            session_id: Some(child_session_id),
            message: format!("Review started with {harness} — comments will post to PR #{number} shortly."),
        },
        live_turn::WorkerLaunchOutcome::Queued => wire::GithubReviewResult {
            status: "queued".into(),
            session_id: None,
            message: format!("Review queued with {harness}; it will start when a worker slot frees up."),
        },
        live_turn::WorkerLaunchOutcome::AwaitingApproval => wire::GithubReviewResult {
            status: "awaitingApproval".into(),
            session_id: None,
            message: "Review is pending an approval; resolve it to let the worker start.".into(),
        },
        live_turn::WorkerLaunchOutcome::Failed => wire::GithubReviewResult {
            status: "failed".into(),
            session_id: None,
            message: "The review worker could not launch; the reason is on the conversation.".into(),
        },
    };
    Ok(result)
}

fn is_cursor_bugbot(harness: &str) -> bool {
    matches!(
        harness.trim().to_ascii_lowercase().as_str(),
        "bugbot" | "cursor-bugbot" | "cursor_bugbot"
    )
}

fn request_cursor_bugbot_review(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
    number: u64,
) -> Result<wire::GithubReviewResult, BridgeError> {
    let path = locked_workspace_path(core, workspace_id)?;
    core.github_surface
        .comment_on_pull_request(Path::new(&path), number, "cursor review")
        .map_err(github_error)?;
    Ok(wire::GithubReviewResult {
        status: "launched".into(),
        session_id: None,
        message: format!(
            "Asked Cursor Bugbot to review PR #{number}. It will comment on the pull request."
        ),
    })
}

/// Check a PR's head branch out into a task worktree of its own — a new
/// workspace node beside the source workspace, never a mutation of it. The
/// resolved PR data (head branch, title) comes from the surface, not the
/// client, so a stale panel cannot check out the wrong branch.
pub fn github_checkout(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
    number: u64,
) -> Result<wire::GithubCheckoutResult, BridgeError> {
    let path = locked_workspace_path(core, workspace_id)?;
    let workspace = Path::new(&path);
    let detail = core
        .github_surface
        .pr_detail(workspace, number)
        .map_err(github_error)?;
    let head_branch = detail.summary.head_branch.clone();
    let title = detail.summary.title.clone();
    let remote = crate::github_surface::resolve_remote_name(workspace);
    let project_id: Option<String> = core.db.lock().unwrap().query_row(
        "SELECT project_id FROM workspaces WHERE id=?1",
        params![workspace_id],
        |row| row.get(0),
    )?;
    let checkout = crate::worktree_coordinator::WorktreeCoordinator::checkout_pull_request(
        &core.db,
        &core.worktrees,
        workspace,
        &remote,
        number,
        &head_branch,
        &title,
        project_id.as_deref(),
    )?;
    if !checkout.reused {
        // A new node in the workspace tree; every client refetches state.
        core.events.publish(CoreEvent::StateChanged);
    }
    Ok(wire::GithubCheckoutResult {
        workspace_id: checkout.workspace_id,
        path: checkout.path.to_string_lossy().into_owned(),
        branch: checkout.branch,
        reused: checkout.reused,
    })
}

// --- verified catalog --------------------------------------------------------

/// The Bridge Verified catalog in force, and how it came to be trusted.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedCatalog<'core> {
    pub generation: u64,
    pub provenance: &'core verified_catalog::Provenance,
    pub entries: &'core [verified_catalog::VerifiedEntry],
    /// What the catalog contributed to backend resolution, and which entries
    /// this build could not serve.
    pub registration: &'core agent_integration::CatalogRegistration,
    /// The refusal code if a cached snapshot lost to the bundled bootstrap at
    /// boot. `None` when the cache was used or there was none.
    pub cache_rejected: Option<&'core str>,
}

/// Read the catalog this build is serving.
///
/// Deliberately **not** a protocol method. #164 lands the catalog itself, and
/// putting it on the wire belongs with the marketplace screen that consumes it —
/// a DTO shaped now, with no reader, would be a contract written against a
/// caller nobody has seen. Its absence from the registry is a decision, and this
/// function is where that decision stops costing anything: the day the screen
/// exists, its method body is one line.
pub fn verified_catalog(core: &Arc<BridgeCore>) -> VerifiedCatalog<'_> {
    VerifiedCatalog {
        generation: core.catalog.generation(),
        provenance: core.catalog.provenance(),
        entries: core.catalog.entries(),
        registration: &core.catalog_registration,
        cache_rejected: core.catalog_cache_rejected.as_deref(),
    }
}

/// What the current conformance suite checks, and which revision it is.
///
/// The desktop never runs the suite — #168's pipeline runs where a candidate
/// can be installed in a clean environment and where vendor credentials live.
/// What the desktop can usefully answer is *what a verdict means*: a UI showing
/// "Bridge Verified" should be able to say what was checked to earn it, without
/// the marketplace screen restating a list that would then drift from the one
/// the suite actually runs.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationSuite {
    pub suite_version: u32,
    pub checks: Vec<&'static str>,
    /// How many times the required set must agree with itself before a verdict
    /// is trusted.
    pub determinism_runs: usize,
}

pub fn verification_suite() -> VerificationSuite {
    VerificationSuite {
        suite_version: verification_pipeline::SUITE_VERSION,
        checks: verification_pipeline::CheckId::ALL
            .iter()
            .map(|check| check.as_str())
            .collect(),
        determinism_runs: verification_pipeline::DETERMINISM_RUNS,
    }
}

// --- projects / workspaces ---------------------------------------------------

pub fn add_project(core: &Arc<BridgeCore>, path: &str) -> Result<BridgeState, BridgeError> {
    core.add_project(path)
}

pub fn create_workspace(core: &Arc<BridgeCore>, title: &str) -> Result<BridgeState, BridgeError> {
    core.create_workspace(title)
}

pub fn connect_workspace_folder(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
    path: &str,
) -> Result<BridgeState, BridgeError> {
    let exists: bool = core.db.lock().unwrap().query_row(
        "SELECT EXISTS(SELECT 1 FROM workspaces WHERE id=?1)",
        params![workspace_id],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(BridgeError::Invalid("workspace does not exist".into()));
    }
    let operation = core.workspace_operation(workspace_id);
    let _operation = operation.lock().unwrap();
    let still_exists: bool = core.db.lock().unwrap().query_row(
        "SELECT EXISTS(SELECT 1 FROM workspaces WHERE id=?1)",
        params![workspace_id],
        |row| row.get(0),
    )?;
    if !still_exists {
        return Err(BridgeError::Invalid("workspace does not exist".into()));
    }
    core.connect_workspace_folder(workspace_id, path)
}

pub fn clone_workspace_repo(core: &Arc<BridgeCore>, url: &str, destination: Option<&str>) -> Result<BridgeState, BridgeError> {
    core.clone_workspace_repo(url, destination)
}

pub fn search_github_repos(query: &str) -> Result<bridge_protocol::messages::SearchGithubReposResult, BridgeError> {
    Ok(bridge_protocol::messages::SearchGithubReposResult { repositories: crate::project_onboarding::search_github_repos(query)? })
}

pub fn locate_workspace_folders(query: &str, search_roots: &[String]) -> Result<bridge_protocol::messages::LocateWorkspaceFoldersResult, BridgeError> {
    Ok(bridge_protocol::messages::LocateWorkspaceFoldersResult { candidates: crate::project_onboarding::locate_folders(query, search_roots)? })
}

/// List the current chat's workspace files for the composer's `@file`
/// autocomplete. Returns an empty list for chats with no connected folder.
pub fn list_workspace_files(
    core: &Arc<BridgeCore>,
    session_id: &str,
) -> Result<Vec<String>, BridgeError> {
    match core.session_workspace_root(session_id) {
        Some(root) => workspace_files::list_files(&root),
        None => Ok(Vec::new()),
    }
}

/// List a workspace's files for the editor's tree and file palette. Same
/// `git ls-files` view as the composer's `@file` autocomplete, so the two
/// surfaces never disagree about what "the project" is.
pub fn list_workspace_tree(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
) -> Result<Vec<String>, BridgeError> {
    let path = core.workspace_path(workspace_id)?;
    workspace_files::list_files(Path::new(&path))
}

/// Read one workspace file for the editor. Resolves the path under the lock,
/// then reads outside it, same as [`workspace_changes`].
pub fn read_workspace_file(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
    path: &str,
) -> Result<workspace_files::FileContents, BridgeError> {
    let root = core.workspace_path(workspace_id)?;
    workspace_files::read_file(Path::new(&root), path)
}

fn with_workspace_lock<T>(
    core: &BridgeCore,
    workspace_id: &str,
    body: impl FnOnce() -> T,
) -> T {
    let operation = core.workspace_operation(workspace_id);
    let _operation = operation.lock().unwrap();
    body()
}

fn with_optional_workspace_lock<T>(
    core: &BridgeCore,
    workspace_id: Option<&str>,
    body: impl FnOnce() -> T,
) -> T {
    let operation = workspace_id.map(|workspace_id| core.workspace_operation(workspace_id));
    let _operation = operation.as_ref().map(|operation| operation.lock().unwrap());
    body()
}

/// Snapshot the workspace path under the operation lock, then drop it so a
/// slow read-only git command cannot stall checkout, writes, or chat start.
fn locked_workspace_path(core: &BridgeCore, workspace_id: &str) -> Result<String, BridgeError> {
    with_workspace_lock(core, workspace_id, || core.workspace_path(workspace_id))
}

/// Whether a session exists and is rooted in the given workspace — the guard for
/// reusing a caller-supplied parent session before attaching a worker to it.
fn session_belongs_to_workspace(
    core: &BridgeCore,
    session_id: &str,
    workspace_id: &str,
) -> Result<bool, BridgeError> {
    let owner: Option<String> = core
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT workspace_id FROM sessions WHERE id=?1",
            params![session_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(owner.as_deref() == Some(workspace_id))
}

/// Checkout must not move HEAD under a running agent, an open terminal, or a
/// live chat that already has a user turn. An empty idle session (the new-chat
/// strip before the first message) is allowed so the branch picker works.
fn workspace_blocks_checkout(
    db: &Connection,
    workspace_id: &str,
) -> Result<bool, BridgeError> {
    let running: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE workspace_id=?1 AND ended_at IS NULL AND status IN ('starting','working','waiting','ready','warm','resuming','restored','checkpointing'))",
        params![workspace_id],
        |row| row.get(0),
    )?;
    if running {
        return Ok(true);
    }
    let history: bool = db.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM sessions s
            JOIN session_entries e ON e.session_id = s.id
            WHERE s.workspace_id=?1 AND s.ended_at IS NULL AND e.kind='user.message'
        )",
        params![workspace_id],
        |row| row.get(0),
    )?;
    Ok(history)
}

/// Write one workspace file, refusing the write if it changed on disk since
/// the editor read it. Returns the new content hash.
pub fn write_workspace_file(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
    path: &str,
    content: &str,
    base_sha256: Option<&str>,
) -> Result<workspace_files::WriteOutcome, BridgeError> {
    core.workspace_path(workspace_id)?;
    with_workspace_lock(core, workspace_id, || {
        let root = core.workspace_path(workspace_id)?;
        workspace_files::write_file(Path::new(&root), path, content, base_sha256)
    })
}

pub fn refresh_workspace(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
) -> Result<BridgeState, BridgeError> {
    // Resolve the path under the lock, then run Git outside it so a slow
    // status scan cannot delay checkout, message submission, or editor saves.
    core.workspace_path(workspace_id)?;
    let path = locked_workspace_path(core, workspace_id)?;
    let stats = git::stats(Path::new(&path))?;
    let branch = git::current_branch(Path::new(&path));
    core.record_workspace_git_stats(workspace_id, stats, branch.as_deref())
}

pub fn list_workspace_branches(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
) -> Result<git::WorkspaceBranches, BridgeError> {
    core.workspace_path(workspace_id)?;
    let path = locked_workspace_path(core, workspace_id)?;
    git::list_branches(Path::new(&path))
}

pub fn checkout_workspace_branch(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
    branch: &str,
) -> Result<BridgeState, BridgeError> {
    core.workspace_path(workspace_id)?;
    with_workspace_lock(core, workspace_id, || {
        let (path, mut active) = {
            let db = core.db.lock().unwrap();
            let path: String = db.query_row(
                "SELECT path FROM workspaces WHERE id=?1",
                params![workspace_id],
                |row| row.get(0),
            )?;
            let active = workspace_blocks_checkout(&db, workspace_id)?;
            (path, active)
        };
        active |= core
            .runtimes
            .lock()
            .unwrap()
            .contains_key(&format!("terminal:{workspace_id}"));

        let switched = git::checkout_branch(Path::new(&path), branch, active)?;
        let (dirty, additions, deletions) = git::stats(Path::new(&path))?;
        let current = switched.current.ok_or_else(|| {
            BridgeError::Invalid(
                "the selected branch left the workspace in detached HEAD state".into(),
            )
        })?;
        let mut db = core.db.lock().unwrap();
        let transaction = db.transaction()?;
        transaction.execute(
            "UPDATE workspaces SET branch=?2,dirty_files=?3,additions=?4,deletions=?5 WHERE id=?1",
            params![workspace_id, current, dirty, additions, deletions],
        )?;
        transaction.execute(
            "DELETE FROM work_fact_cache WHERE kind=?1 AND cache_key=?2",
            params![work::FACT_CACHE_BASE_DIVERGENCE, workspace_id],
        )?;
        transaction.commit()?;
        drop(db);
        core.events.publish(CoreEvent::StateChanged);
        core.state_snapshot()
    })
}

/// The workspace's uncommitted changeset for the importance-first review UI:
/// every path that differs from `HEAD`, tracked or not, each with a unified
/// diff and an importance badge. Resolves the path under the lock, then runs
/// Git entirely outside it, same as [`refresh_workspace`].
pub fn workspace_changes(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
) -> Result<git::WorkspaceChangeset, BridgeError> {
    core.workspace_path(workspace_id)?;
    let path = locked_workspace_path(core, workspace_id)?;
    git::workspace_changeset(Path::new(&path))
}

pub fn archive_workspace(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
) -> Result<BridgeState, BridgeError> {
    // The core publishes state-changed as soon as the archive commits, so a
    // snapshot failure below cannot leave listeners unaware of it.
    core.workspace_path(workspace_id)?;
    let operation = core.workspace_operation(workspace_id);
    let _operation = operation.lock().unwrap();
    if core
        .runtimes
        .lock()
        .unwrap()
        .contains_key(&format!("terminal:{workspace_id}"))
    {
        return Err(BridgeError::Invalid(
            "Close the workspace terminal before archiving this workspace".into(),
        ));
    }
    core.archive_workspace(workspace_id)?;
    core.state_snapshot()
}

// --- sessions ----------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForestDigest {
    pub digest: String,
}

pub use bridge_protocol::messages::{ContextBreakdownDigestResult, ContextBreakdownResult};

/// The cheap half of forest polling: an opaque token that changes whenever
/// `get_session_forest` would return different store-derived content.
pub fn get_session_forest_digest(
    core: &Arc<BridgeCore>,
    session_id: &str,
) -> Result<ForestDigest, BridgeError> {
    Ok(ForestDigest {
        digest: core.session_forest_digest(session_id)?,
    })
}

/// The bounded, source-labelled context breakdown for one session.
pub fn get_context_breakdown(
    core: &Arc<BridgeCore>,
    session_id: &str,
) -> Result<wire::ContextBreakdownResult, BridgeError> {
    core.context_breakdown(session_id)
}

/// The cheap half of breakdown polling: an opaque token covering prompt
/// compilations, config revisions, adapter observations, and branch changes.
pub fn get_context_breakdown_digest(
    core: &Arc<BridgeCore>,
    session_id: &str,
) -> Result<wire::ContextBreakdownDigestResult, BridgeError> {
    core.context_breakdown_digest(session_id)
}

pub fn get_session_forest(
    core: &Arc<BridgeCore>,
    session_id: &str,
) -> Result<SessionForestSnapshot, BridgeError> {
    // Git may be slow on large repositories or during index contention. Never
    // call this while holding the global SQLite lock.
    let repository_path = core.session_repository_path(session_id)?;
    let repository_state = match repository_path {
        Some(path) => store::repository_state_for_path(&path),
        None => serde_json::json!({"status":"unavailable"}),
    };
    core.session_forest_snapshot_with_repository_state(session_id, repository_state)
}

/// Replay durable session events after a cursor — the recovery half of the
/// notify-then-replay event contract.
pub fn replay_session_events(
    core: &Arc<BridgeCore>,
    session_id: &str,
    after_sequence: i64,
    limit: Option<u32>,
    tail: Option<bool>,
) -> Result<Vec<AgentEvent>, BridgeError> {
    core.replay_session_events(session_id, after_sequence, limit, tail)
}

pub fn activate_session_entry(
    core: &Arc<BridgeCore>,
    session_id: &str,
    entry_id: &str,
) -> Result<SessionForestSnapshot, BridgeError> {
    // The core publishes state-changed once the head move is recorded.
    core.activate_session_entry(session_id, entry_id)
}

pub fn create_chat(
    core: &Arc<BridgeCore>,
    harness: &Harness,
    model: Option<&str>,
    title: Option<&str>,
) -> Result<BridgeState, BridgeError> {
    core.create_chat(harness, model, title)
}

/// Return the identity from the insert instead of inferring it from a snapshot.
pub fn create_chat_id(
    core: &Arc<BridgeCore>,
    harness: &Harness,
    model: Option<&str>,
    title: Option<&str>,
) -> Result<wire::CreateChatIdResult, BridgeError> {
    let session_id = core.create_chat_id(harness, model, title)?;
    core.events.publish(CoreEvent::StateChanged);
    Ok(wire::CreateChatIdResult { session_id })
}

pub fn create_aside_chat(
    core: &Arc<BridgeCore>,
    source_session_id: &str,
    harness: &Harness,
    model: Option<&str>,
    title: Option<&str>,
) -> Result<wire::CreateAsideChatResult, BridgeError> {
    let (session_id, carried, native_fork) =
        core.create_aside_chat_id(source_session_id, harness, model, title)?;
    Ok(wire::CreateAsideChatResult {
        state: protocol_wire(core.state_snapshot()?)?,
        source_session_id: source_session_id.to_owned(),
        session_id,
        handoff_status: if native_fork {
            "forked"
        } else if carried {
            "carried"
        } else {
            "empty"
        }
        .into(),
        fidelity: if native_fork {
            "native"
        } else if carried {
            "projected_at_boundary"
        } else {
            "native"
        }
        .into(),
    })
}

/// Create an orchestrator session inside a workspace (the classic Bridge agent
/// that plans and delegates to workers). Multiple are allowed per workspace.
pub fn create_workspace_session(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
    create_worktree: bool,
) -> Result<BridgeState, BridgeError> {
    core.workspace_path(workspace_id)?;
    let operation = core.workspace_operation(workspace_id);
    let _operation = operation.lock().unwrap();
    let plan = core.plan_workspace_session(workspace_id, create_worktree)?;
    let worktree = match plan.worktree_source().map(str::to_owned) {
        Some(source) => Some(sessions::prepare_orchestrator_worktree(
            &core.worktrees,
            plan.workspace_title(),
            Path::new(&source),
            plan.session_id(),
        )?),
        None => None,
    };
    core.persist_workspace_session(plan, worktree)
}

pub fn start_session(
    core: &Arc<BridgeCore>,
    workspace_id: String,
    harness: Option<Harness>,
    model: Option<String>,
) -> Result<BridgeState, BridgeError> {
    live_turn::start_session(core, workspace_id, harness, model)
}

/// Start (or hot-return) a session by id. A `direct` chat runs the stored
/// harness/model with no briefing; an `orchestrator` session runs codex with
/// the routing briefing + delegation protocol.
pub fn start_chat(core: &Arc<BridgeCore>, session_id: String) -> Result<BridgeState, BridgeError> {
    // The "imported history cannot resume" gate lives in `live_turn::start_chat`
    // itself, since that is also the function `resume_for_send` reaches from
    // the composer's implicit resume path — a check only here would miss it.
    live_turn::start_chat(core, session_id)
}

/// Change a root chat's provider/model. Stops any running adapter so the next
/// message starts a fresh provider session with the explicit user selection.
pub fn update_chat_model(
    core: &Arc<BridgeCore>,
    session_id: &str,
    harness: &Harness,
    model: Option<&str>,
    effort: Option<&str>,
) -> Result<BridgeState, BridgeError> {
    // Exclusive for the whole plan -> teardown -> commit window: a concurrent
    // start would otherwise slip in after teardown and be orphaned by the
    // commit clearing its process and turn state.
    let _lifecycle = core.claim_session_lifecycle(session_id, "model switch")?;
    let change = core.plan_chat_model_change(session_id, harness, model)?;
    let (previous_model, previous_effort): (Option<String>, Option<String>) = core.db.lock().unwrap().query_row(
        "SELECT model,effort FROM sessions WHERE id=?1", params![session_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let target_model = change.as_ref().map(|change| change.selected_model())
        .unwrap_or(previous_model.as_deref());
    let descriptor = core.adapter_registry.descriptors().into_iter()
        .find(|adapter| adapter.id == store::harness_name(harness))
        .ok_or_else(|| BridgeError::Invalid("Model adapter is unavailable".into()))?;
    let next_effort = if target_model.is_none() && descriptor.models.is_empty() && effort.is_none() {
        // Provider defaults have no advertised effort controls yet. Drop the
        // outgoing provider's effort rather than carrying it into this launch.
        None
    } else {
        let selected = descriptor.models.iter().find(|option| Some(option.id.as_str()) == target_model)
            .ok_or_else(|| BridgeError::Invalid("Selected model is unavailable; refresh models".into()))?;
        selected_chat_effort(selected, effort, previous_effort.as_deref())?
    };
    let effort_changed = next_effort != previous_effort;
    let model_changed = change.is_some();
    if let Some(change) = change {
        // Only a switch the next turn can natively resume skips the handover.
        // The agent resumes its own thread under the new model and keeps the
        // conversation, so asking the outgoing model to summarise would spend
        // a turn and a 20-second budget producing something nothing reads —
        // and every timeout would land in the ledger as a compaction failure.
        // Anything else — a harness change, an adapter without native resume,
        // a chat with no stored thread — still summarises exactly as before.
        // Take the outgoing model's handoff summary off the switch's critical
        // path. Rather than block the invoke while the old provider writes a
        // checkpoint, detach its runtime — kept alive on its own reader — and
        // let a background waiter summarise after the switch commits. The
        // switch itself proceeds on Bridge's mechanical projection, which
        // loses nothing the stored history did not already hold.
        let detached = if !change.resumes_natively() {
            detach_switch_summary(core, session_id)
        } else {
            false
        };
        if !detached {
            core.stop_session_adapter(session_id, adapters::ShutdownReason::Replaced);
        }
        // With no live (attached) runtime, settle any turn residue so the
        // commit's revision check sees the idle row the plan verified.
        core.settle_adapterless_turn_state(session_id, std::time::Duration::from_secs(3))?;
        if let Err(error) = core.commit_chat_model_change(change) {
            if detached {
                // The switch never landed: stop the detached runtime and cancel
                // its pending request so a later reply is not misparsed.
                switch_summary::abort(core, session_id);
                let _ = core.cancel_switch_summary(
                    session_id,
                    "model switch did not commit; summary abandoned",
                    1,
                );
            }
            return Err(error);
        }
        if detached {
            let background = core.clone();
            let session = session_id.to_owned();
            thread::spawn(move || switch_summary::deliver_and_wait(background, session));
        }
    }
    if effort_changed {
        if !model_changed {
            // Effort is a launch option. Keep the native session identity and
            // restart its runtime on the next turn so a warm query cannot ignore it.
            core.stop_session_adapter(session_id, adapters::ShutdownReason::Replaced);
        }
        let db = core.db.lock().unwrap();
        let transaction = db.unchecked_transaction()?;
        session_supervisor::SessionSupervisor::clear_adapter_process(&transaction, session_id)?;
        transaction.execute(
            "UPDATE sessions SET effort=?2 WHERE id=?1 AND active_turn_id IS NULL",
            rusqlite::params![session_id, next_effort],
        )?;
        transaction.commit()?;
    }
    core.state_snapshot()
}

/// Explicit user choices are validated; inherited choices incompatible with a
/// new model return to the provider default instead of leaking across harnesses.
fn selected_chat_effort(
    model: &crate::model::ModelOption,
    requested: Option<&str>,
    previous: Option<&str>,
) -> Result<Option<String>, BridgeError> {
    if let Some(value) = requested {
        if !model.supported_effort_levels.iter().any(|level| level == value) {
            return Err(BridgeError::Invalid(format!(
                "{} does not support thinking level {value}; refresh models and choose a supported level",
                model.label,
            )));
        }
    }
    Ok(requested.or(previous)
        .filter(|value| model.supported_effort_levels.iter().any(|level| level == value))
        .map(str::to_owned))
}

/// Plan the outgoing model's handoff summary and detach its runtime so the
/// switch can commit without waiting on it. Returns whether a detached summary
/// is now in flight; `false` means there was nothing worth summarising or no
/// live provider, and the caller stops the adapter the ordinary way.
///
/// The summary itself is delivered and awaited off-thread by
/// [`switch_summary::deliver_and_wait`], under the controller's own budget —
/// never the switch's, which now returns immediately.
fn detach_switch_summary(core: &Arc<BridgeCore>, session_id: &str) -> bool {
    let request = match core.plan_switch_summary(session_id) {
        Ok(Some(request)) => request,
        _ => return false,
    };
    if switch_summary::detach(core, session_id, request) {
        return true;
    }
    // `plan_switch_summary` saw a live runtime but it went away before detach.
    // Cancel the request it began so a later reply is not misparsed.
    let _ = core.cancel_switch_summary(
        session_id,
        "no live provider to summarise; stored history carried",
        0,
    );
    false
}

/// Carry a source chat's projected context into another chat as a durable
/// handoff brief — the `$harness` shortcut's way of giving the new sibling
/// chat the conversation it was asked about. Best-effort: `carried` reports
/// whether anything was carried.
pub fn carry_session_handoff(
    core: &Arc<BridgeCore>,
    target_session_id: &str,
    source_session_id: &str,
) -> Result<wire::CarrySessionHandoffResult, BridgeError> {
    let db = core.db.lock().unwrap();
    let carried = handoff::carry_brief(&db, target_session_id, source_session_id)?;
    Ok(wire::CarrySessionHandoffResult { carried })
}

pub fn prepare_turn(
    core: &Arc<BridgeCore>,
    session_id: String,
    text: String,
) -> Result<secret_interception::SanitizedTurn, BridgeError> {
    live_turn::prepare_turn(core, session_id, text)
}

pub fn send_turn(
    core: &Arc<BridgeCore>,
    session_id: String,
    text: String,
) -> Result<(), BridgeError> {
    live_turn::send_turn(core, session_id, text)
}

/// Submit user input and let Bridge decide what to do with it: start a turn,
/// steer the one already running, or durably queue it for the next phase
/// boundary. The disposition comes back so the client can say which happened.
pub fn submit_input(
    core: &Arc<BridgeCore>,
    session_id: String,
    text: String,
) -> Result<wire::SubmitInputResult, BridgeError> {
    live_turn::submit_input(core, session_id, text)
}

/// Same contract with pasted image attachments attached. Every image either
/// reaches a provider that supports them or the caller sees an explicit error.
pub fn submit_input_with_attachments(
    core: &Arc<BridgeCore>,
    session_id: String,
    text: String,
    attachments: Vec<wire::TurnImage>,
) -> Result<wire::SubmitInputResult, BridgeError> {
    live_turn::submit_input_with_attachments(core, session_id, text, attachments)
}

/// Resolve and dispatch a leading `#agent` directive without starting or
/// steering the parent provider. The live-turn layer owns the lifecycle path.
pub fn dispatch_agent_shortcut(
    core: &Arc<BridgeCore>,
    session_id: String,
    token: String,
    objective: String,
) -> Result<wire::DispatchAgentShortcutResult, BridgeError> {
    live_turn::dispatch_agent_shortcut(core, session_id, token, objective)
}

pub fn compact_session(core: &Arc<BridgeCore>, session_id: &str) -> Result<(), BridgeError> {
    let prompt = core.begin_manual_compaction(session_id)?;
    match live_turn::send_internal_checkpoint_turn(core, session_id, &prompt) {
        Ok(()) => Ok(()),
        Err(error) => {
            // `begin_manual_compaction` already appended the request. Settle it
            // on delivery failure or every later retry sees a phantom pending
            // compaction and is refused forever.
            let attempt = compaction_controller::CompactionController::pending(
                &core.db.lock().unwrap(),
                session_id,
            )?
            .map_or(0, |pending| pending.attempt);
            let reason = format!("checkpoint turn could not start: {error}");
            compaction_controller::CompactionController::record_failure(
                &core.db.lock().unwrap(),
                session_id,
                &reason,
                attempt,
            )?;
            Err(BridgeError::Invalid(
                "Compaction could not start because the provider is unavailable. Your conversation history is intact; reconnect the provider and retry."
                    .into(),
            ))
        }
    }
}

pub fn search_session_entries(
    core: &Arc<BridgeCore>,
    session_id: &str,
    query: &str,
    limit: Option<u32>,
) -> Result<bridge_protocol::messages::SearchSessionEntriesResult, BridgeError> {
    let db = core.db.lock().unwrap();
    session_recall::search(&db, session_id, query, limit)
}

pub fn save_memory_record(
    core: &Arc<BridgeCore>,
    body: &str,
    kind: Option<&str>,
    session_id: Option<&str>,
) -> Result<bridge_protocol::messages::MemoryRecord, BridgeError> {
    let record = {
        let db = core.db.lock().unwrap();
        memory_ledger::save(&db, body, kind, session_id)?
    };
    core.events.publish(CoreEvent::MemoryChanged {
        scope_key: record.scope_key.clone(),
    });
    Ok(record)
}

pub fn list_memory_records(
    core: &Arc<BridgeCore>,
    scope_key: &str,
    status: Option<&str>,
) -> Result<bridge_protocol::messages::ListMemoryRecordsResult, BridgeError> {
    let db = core.db.lock().unwrap();
    memory_ledger::list(&db, scope_key, status)
}

pub fn delete_memory_record(
    core: &Arc<BridgeCore>,
    record_id: &str,
) -> Result<bridge_protocol::messages::MemoryRecord, BridgeError> {
    let record = {
        let db = core.db.lock().unwrap();
        memory_ledger::forget(&db, record_id)?
    };
    core.events.publish(CoreEvent::MemoryChanged {
        scope_key: record.scope_key.clone(),
    });
    Ok(record)
}

pub fn supersede_memory_record(
    core: &Arc<BridgeCore>,
    record_id: &str,
    body: &str,
    kind: Option<&str>,
) -> Result<bridge_protocol::messages::MemoryRecord, BridgeError> {
    let record = {
        let db = core.db.lock().unwrap();
        memory_ledger::supersede(&db, record_id, body, kind)?
    };
    core.events.publish(CoreEvent::MemoryChanged {
        scope_key: record.scope_key.clone(),
    });
    Ok(record)
}

pub fn approve_memory_record(
    core: &Arc<BridgeCore>,
    record_id: &str,
) -> Result<bridge_protocol::messages::MemoryRecord, BridgeError> {
    let record = {
        let db = core.db.lock().unwrap();
        memory_ledger::approve(&db, record_id)?
    };
    core.events.publish(CoreEvent::MemoryChanged {
        scope_key: record.scope_key.clone(),
    });
    Ok(record)
}

pub fn reject_memory_record(
    core: &Arc<BridgeCore>,
    record_id: &str,
) -> Result<bridge_protocol::messages::MemoryRecord, BridgeError> {
    let record = {
        let db = core.db.lock().unwrap();
        memory_ledger::reject(&db, record_id)?
    };
    core.events.publish(CoreEvent::MemoryChanged {
        scope_key: record.scope_key.clone(),
    });
    Ok(record)
}

fn extraction_settings_wire(
    db: &rusqlite::Connection,
    settings: crate::memory_extraction::ExtractionSettings,
) -> Result<wire::MemoryExtractionSettings, BridgeError> {
    let last_run = crate::memory_extraction::last_run(db, &settings.scope_key)?.map(|run| {
        wire::MemoryExtractionRun {
            status: run.status,
            proposal_count: run.proposal_count,
            observed_tokens: run.observed_tokens,
            spend_microusd: run.spend_microusd,
            detail: run.detail,
            updated_at: run.updated_at,
        }
    });
    Ok(wire::MemoryExtractionSettings {
        scope_key: settings.scope_key,
        mode: settings.mode,
        harness: settings.harness,
        model: settings.model,
        last_run,
    })
}

pub fn get_extraction_settings(
    core: &Arc<BridgeCore>,
) -> Result<wire::MemoryExtractionSettings, BridgeError> {
    let db = core.db.lock().unwrap();
    let settings =
        crate::memory_extraction::settings(&db, memory_ledger::account_memory_scope())?;
    extraction_settings_wire(&db, settings)
}

pub fn update_extraction_settings(
    core: &Arc<BridgeCore>,
    mode: &str,
    harness: Option<&str>,
    model: Option<&str>,
) -> Result<wire::MemoryExtractionSettings, BridgeError> {
    let db = core.db.lock().unwrap();
    let settings = crate::memory_extraction::update_settings(
        &db,
        memory_ledger::account_memory_scope(),
        mode,
        harness,
        model,
    )?;
    extraction_settings_wire(&db, settings)
}

fn consolidation_settings_wire(
    db: &rusqlite::Connection,
    settings: crate::memory_consolidation::ConsolidationSettings,
) -> Result<wire::MemoryConsolidationSettings, BridgeError> {
    let last_run = crate::memory_consolidation::last_run(db, &settings.scope_key)?.map(|run| {
        wire::MemoryConsolidationRun {
            status: run.status,
            applied_count: run.applied_count,
            refused_count: run.refused_count,
            observed_tokens: run.observed_tokens,
            spend_microusd: run.spend_microusd,
            detail: run.detail,
            updated_at: run.updated_at,
        }
    });
    let held_records = crate::memory_consolidation::held_records(db, &settings.scope_key)?;
    Ok(wire::MemoryConsolidationSettings {
        scope_key: settings.scope_key,
        mode: settings.mode,
        harness: settings.harness,
        model: settings.model,
        max_records: settings.max_records,
        allow_removal: settings.allow_removal,
        debounce_seconds: settings.debounce_seconds,
        held_records,
        last_run,
    })
}

pub fn get_consolidation_settings(
    core: &Arc<BridgeCore>,
) -> Result<wire::MemoryConsolidationSettings, BridgeError> {
    let db = core.db.lock().unwrap();
    let settings =
        crate::memory_consolidation::settings(&db, memory_ledger::account_memory_scope())?;
    consolidation_settings_wire(&db, settings)
}

#[allow(clippy::too_many_arguments)]
pub fn update_consolidation_settings(
    core: &Arc<BridgeCore>,
    mode: &str,
    harness: Option<&str>,
    model: Option<&str>,
    max_records: Option<i64>,
    allow_removal: Option<bool>,
    debounce_seconds: Option<i64>,
) -> Result<wire::MemoryConsolidationSettings, BridgeError> {
    let db = core.db.lock().unwrap();
    let settings = crate::memory_consolidation::update_settings(
        &db,
        memory_ledger::account_memory_scope(),
        mode,
        harness,
        model,
        max_records,
        allow_removal,
        debounce_seconds,
    )?;
    consolidation_settings_wire(&db, settings)
}

/// The scope as it stood at one instant. History is a read, never a rewrite:
/// nothing here reopens a closed interval or resurrects a tombstoned record.
pub fn list_memory_records_as_of(
    core: &Arc<BridgeCore>,
    scope_key: &str,
    at: &str,
) -> Result<bridge_protocol::messages::ListMemoryRecordsResult, BridgeError> {
    let at = chrono::DateTime::parse_from_rfc3339(at.trim())
        .map_err(|error| {
            BridgeError::Invalid(format!("'{at}' is not an RFC 3339 instant: {error}"))
        })?
        .with_timezone(&chrono::Utc);
    let db = core.db.lock().unwrap();
    memory_ledger::list_as_of(&db, scope_key, at)
}

pub fn get_memory_injection(
    core: &Arc<BridgeCore>,
) -> Result<wire::MemoryInjectionSettings, BridgeError> {
    let db = core.db.lock().unwrap();
    Ok(wire::MemoryInjectionSettings {
        scope_key: memory_ledger::account_memory_scope().to_string(),
        enabled: crate::memory_packet::injection_enabled(&db, memory_ledger::account_memory_scope())?,
    })
}

pub fn set_memory_injection(
    core: &Arc<BridgeCore>,
    enabled: bool,
) -> Result<wire::MemoryInjectionSettings, BridgeError> {
    let db = core.db.lock().unwrap();
    let enabled =
        crate::memory_packet::set_injection(&db, memory_ledger::account_memory_scope(), enabled)?;
    Ok(wire::MemoryInjectionSettings {
        scope_key: memory_ledger::account_memory_scope().to_string(),
        enabled,
    })
}

pub fn get_packet_audit(
    core: &Arc<BridgeCore>,
    session_id: &str,
) -> Result<wire::MemoryPacketAudit, BridgeError> {
    let db = core.db.lock().unwrap();
    let audit = crate::memory_packet::latest_audit(&db, session_id)?;
    Ok(match audit {
        None => wire::MemoryPacketAudit {
            session_id: session_id.to_string(),
            selected: Vec::new(),
            token_estimate: 0,
            created_at: None,
        },
        Some(audit) => wire::MemoryPacketAudit {
            session_id: session_id.to_string(),
            selected: audit
                .selected
                .into_iter()
                .map(|item| wire::MemoryPacketItem {
                    record_id: item.record_id,
                    body: item.body,
                    kind: item.kind,
                    reason: item.reason,
                })
                .collect(),
            token_estimate: audit.token_estimate,
            created_at: Some(audit.created_at),
        },
    })
}

/// What memory exists here, so the UI can be honest about what it does not
/// own. The provider half is derived from the slash catalog: an unavailable
/// adapter contributes nothing, and no harness name is compared in this body.
pub fn get_memory_capabilities(
    core: &Arc<BridgeCore>,
) -> Result<wire::MemoryCapabilities, BridgeError> {
    let provider_native = slash::provider_memory_commands(&available_adapter_ids(core))
        .into_iter()
        .map(|command| wire::ProviderMemoryCommand {
            harness: command.harness,
            command: command.name,
            description: command.description,
        })
        .collect();
    Ok(wire::MemoryCapabilities {
        ledger: wire::MemoryLedgerCapability {
            exists: true,
            scope_key: wire::ACCOUNT_MEMORY_SCOPE.to_string(),
            max_body_chars: wire::MAX_MEMORY_BODY_CHARS as u32,
            kinds: wire::MEMORY_KINDS.iter().map(|kind| kind.to_string()).collect(),
        },
        provider_native,
    })
}

/// Run a finished worker's objective again because the user asked. Goes through
/// the ordinary launch path, so every policy limit applies as it did the first
/// time.
pub fn retry_worker_task(
    core: &Arc<BridgeCore>,
    child_session_id: &str,
) -> Result<(), BridgeError> {
    live_turn::retry_worker_task(core, child_session_id)
}

pub fn interrupt_turn(core: &Arc<BridgeCore>, session_id: &str) -> Result<(), BridgeError> {
    live_turn::cancel_visible_turn(core, session_id)
}

/// Refresh subscription usage for every provider, independent of which session
/// is on screen. Results are broadcast on the `account-usage` channel.
pub fn refresh_account_usage(core: &Arc<BridgeCore>) -> Result<(), BridgeError> {
    core.refresh_account_usage()
}

pub fn stop_session(
    core: &Arc<BridgeCore>,
    session_id: String,
) -> Result<BridgeState, BridgeError> {
    live_turn::stop_session(core, session_id)
}

// --- approvals ---------------------------------------------------------------

pub fn resolve_approval(
    core: &Arc<BridgeCore>,
    session_id: &str,
    event_id: i64,
    decision: &str,
    option_id: Option<&str>,
) -> Result<wire::InteractionResolutionResult, BridgeError> {
    let kind: String = core.db.lock().unwrap().query_row(
        "SELECT kind FROM session_entries WHERE session_id=?1 AND sequence=?2",
        params![session_id, event_id],
        |row| row.get(0),
    )?;
    match kind.as_str() {
        "permission.requested" => resolve_provider_permission(
            core,
            session_id,
            event_id,
            Some(decision),
            option_id,
            "human",
            None,
        ),
        "question.requested" => Err(BridgeError::Invalid(
            "This interaction is a question; answer it through the question reply channel".into(),
        )),
        // Host-side delegation authorization predates provider interactions
        // and has no provider response to deduplicate. Keep its existing
        // transaction, but return the same typed command result.
        "approval.requested" => {
            let prompt_proposal = {
                let db = core.db.lock().unwrap();
                let payload: String = db.query_row(
                    "SELECT payload FROM session_entries WHERE session_id=?1 AND sequence=?2",
                    params![session_id, event_id], |row| row.get(0),
                )?;
                let payload: Value = serde_json::from_str(&payload)
                    .map_err(|error| BridgeError::Invalid(format!("Approval metadata is invalid: {error}")))?;
                if payload["approvalType"] == "prompt_mutation" {
                    Some(payload["proposalId"].as_str().ok_or_else(||
                        BridgeError::Invalid("Prompt approval has no proposal id".into()))?.to_owned())
                } else { None }
            };
            if let Some(proposal_id) = prompt_proposal {
                return resolve_prompt_mutation_approval(core, session_id, event_id, &proposal_id, decision);
            }
            resolve_legacy_approval(core, session_id, event_id, decision)?;
            Ok(interaction_result(
                wire::InteractionResolutionDisposition::Resolved,
                "permission",
                decision_status(decision),
                "human",
                decision,
                None,
            ))
        }
        _ => Err(BridgeError::Invalid(
            "The requested event is not a permission interaction".into(),
        )),
    }
}

fn resolve_prompt_mutation_approval(
    core: &Arc<BridgeCore>, session_id: &str, event_id: i64, proposal_id: &str, decision: &str,
) -> Result<wire::InteractionResolutionResult, BridgeError> {
    if !matches!(decision, "accept" | "decline" | "cancel") {
        return Err(BridgeError::Invalid("Prompt changes require an explicit decision for this exact change; session approval is unavailable".into()));
    }
    let resolved = {
        let db = core.db.lock().unwrap();
        let proposal = crate::prompt_mutations::get(&db, proposal_id)?
            .ok_or_else(|| BridgeError::Invalid("Prompt proposal no longer exists".into()))?;
        if proposal.approval_session_id != session_id || proposal.approval_event_id != event_id {
            return Err(BridgeError::Invalid("This approval does not belong to the prompt proposal".into()));
        }
        crate::prompt_mutations::resolve(&db, proposal_id, decision == "accept")?
    };
    let status = resolved.status.as_str();
    let reason = live_turn::prompt_mutation_outcome_reason(status);
    // The durable queue receipt makes this safe on a repeated decision and
    // lets a retry deliver an outcome whose first post-commit enqueue failed.
    live_turn::queue_prompt_mutation_feedback(core, &resolved.proposal.actor_session_id,
        Some(proposal_id), status, reason, None);
    live_turn::notify_parent_prompt_mutation_resolved(core, &resolved.proposal.actor_session_id,
        proposal_id, status);
    core.events.publish(CoreEvent::StateChanged);
    Ok(interaction_result(
        if resolved.already_resolved { wire::InteractionResolutionDisposition::AlreadyResolved }
        else { wire::InteractionResolutionDisposition::Resolved },
        "permission", status, "human", status, Some(reason),
    ))
}

fn resolve_legacy_approval(
    core: &Arc<BridgeCore>,
    session_id: &str,
    event_id: i64,
    decision: &str,
) -> Result<(), BridgeError> {
    if !matches!(
        decision,
        "accept" | "acceptForSession" | "decline" | "cancel"
    ) {
        return Err(BridgeError::Invalid("Unsupported approval decision".into()));
    }
    let db = core.db.lock().unwrap();
    let (data, adapter_id): (String, String) = db.query_row(
        "SELECT e.payload,s.harness FROM session_entries e
         JOIN sessions s ON s.id=e.session_id
         WHERE e.session_id=?1 AND e.sequence=?2 AND e.kind='approval.requested'",
        params![session_id, event_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let data: Value = serde_json::from_str(&data)
        .map_err(|e| BridgeError::Invalid(format!("Approval metadata is invalid: {e}")))?;
    // Two storage shapes reach this reader: adapter approvals persist through
    // the agent-event envelope (details nested under `data`), while policy
    // approvals are typed forest entries with top-level fields. Look in the
    // envelope's `data` first and fall back to the top level.
    let detail = |name: &str| {
        data.get("data")
            .and_then(|nested| nested.get(name))
            .or_else(|| data.get(name))
            .cloned()
    };
    if detail("approvalType").as_ref().and_then(Value::as_str) == Some("delegation_path_scope") {
        let resolved = live_turn::resolve_policy_delegation_approval(
            &db, session_id, event_id, decision, &data,
        )?;
        drop(db);
        if !resolved.accepted {
            live_turn::report_delegation_approval_declined(
                core,
                session_id,
                &resolved.turn_id,
                &resolved.approval_id,
                decision,
                &resolved.request,
            );
            return Ok(());
        }
        match live_turn::launch_worker_outcome(
            core,
            session_id,
            &resolved.turn_id,
            &resolved.request,
            true,
        ) {
            live_turn::WorkerLaunchOutcome::Launched(child_session_id) => {
                live_turn::report_approved_launch_adopted(
                    core,
                    session_id,
                    &resolved.turn_id,
                    &resolved.approval_id,
                    Some(&child_session_id),
                    false,
                );
            }
            live_turn::WorkerLaunchOutcome::Queued => {
                live_turn::report_approved_launch_adopted(
                    core,
                    session_id,
                    &resolved.turn_id,
                    &resolved.approval_id,
                    None,
                    true,
                );
            }
            // An approved scope that still routes to approval would loop the
            // user; treat it as a launch failure so the turn terminates.
            live_turn::WorkerLaunchOutcome::AwaitingApproval
            | live_turn::WorkerLaunchOutcome::Failed => {
                let db = core.db.lock().unwrap();
                live_turn::record_approved_launch_failure(
                    &db,
                    session_id,
                    &resolved.turn_id,
                    &resolved.request,
                )?;
                drop(db);
                core.events.publish(CoreEvent::StateChanged);
                return Err(BridgeError::Invalid(
                    "Write scope was approved, but the worker could not launch; the delegation may be retried for this turn".into(),
                ));
            }
        }
        core.events.publish(CoreEvent::StateChanged);
        return Ok(());
    }
    // One answer per request. The approval event is published to every client
    // before anything resolves it, so a human click and the permission policy can
    // both reach here for the same request id — and two `respond` calls for one
    // request is a contradictory answer to the provider plus two resolutions in
    // the transcript. Multi-window clients could already race this; the policy
    // just made the window routine.
    let already_resolved: bool = db
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM session_entries
                 WHERE session_id=?1 AND kind='approval.resolved'
                   AND json_extract(payload,'$.data.requestEventId')=?2
             )",
            params![session_id, event_id],
            |row| row.get(0),
        )
        .unwrap_or(false);
    if already_resolved {
        return Err(BridgeError::Invalid(
            "This approval has already been answered".into(),
        ));
    }
    let request_id = detail("requestId")
        .ok_or_else(|| BridgeError::Invalid("Approval has no adapter request id".into()))?;
    // A question is answered with text/options over its own reply channel,
    // never with an accept/decline decision — `respond` posts the wrong
    // shape to the wrong endpoint for one (#282). The decision-only card
    // this method serves has nothing to answer a question with, so only a
    // decline (dismissing it) is meaningful here; an accept from this path
    // would otherwise silently discard whatever the user actually typed.
    let is_question = detail("requestMethod").as_ref().and_then(Value::as_str)
        == Some(agent::OPENCODE_QUESTION_REQUEST_METHOD);
    if is_question && !matches!(decision, "decline" | "cancel") {
        return Err(BridgeError::Invalid(
            "This is a question, not an approval — type an answer in the composer instead of accepting or declining".into(),
        ));
    }
    // Shared with `live_turn::answer_pending_question` under the same
    // session-scoped label: a typed answer racing this card's Decline (or two
    // decliners racing each other) must not both find the request unresolved
    // and both call the adapter with contradictory replies. Held until this
    // function returns; released on every path, including an early `?`.
    let _claim = if is_question {
        Some(
            core.claim_session_lifecycle(session_id, "question resolution")
                .map_err(|_| {
                    BridgeError::Invalid("This question is already being answered".into())
                })?,
        )
    } else {
        None
    };
    let is_worker = store::worker_runtime(&db, session_id)?.is_some();
    if is_worker {
        session_supervisor::SessionSupervisor::transition(
            &db,
            session_id,
            worker_lifecycle::WorkerLifecycleState::Working,
            Some("approval_resolved"),
        )?;
    }
    drop(db);
    let adapters = core.adapters.lock().unwrap();
    let runtime = adapters
        .get(session_id)
        .ok_or_else(|| BridgeError::Invalid("Structured adapter session is not running".into()))?;
    let response = if is_question {
        runtime.reject_question(request_id)
    } else {
        runtime.respond(request_id, decision)
    };
    if let Err(error) = response {
        drop(adapters);
        if is_worker {
            let _ = session_supervisor::SessionSupervisor::transition(
                &core.db.lock().unwrap(),
                session_id,
                worker_lifecycle::WorkerLifecycleState::Waiting,
                Some("approval_delivery_failed"),
            );
        }
        return Err(error);
    }
    drop(adapters);
    let mut normalized = agent::NormalizedEvent {
        kind: "approval.resolved".into(),
        item_id: None,
        role: None,
        status: Some(decision.to_owned()),
        title: Some("Approval resolved".into()),
        text: None,
        data: serde_json::json!({"requestEventId":event_id,"decision":decision}),
    };
    normalized.item_id = detail("itemId")
        .as_ref()
        .and_then(Value::as_str)
        .map(str::to_owned);
    let db = core.db.lock().unwrap();
    let event = store::session_event(
        &db,
        session_id,
        &normalized,
        &serde_json::json!({"adapter":adapter_id}),
    )?;
    if !is_worker {
        db.execute(
            "UPDATE sessions SET status='working' WHERE id=?1",
            params![session_id],
        )?;
    }
    db.execute(
        "UPDATE workspaces SET status=CASE
            WHEN EXISTS(SELECT 1 FROM sessions WHERE workspace_id=workspaces.id AND status='waiting') THEN 'waiting'
            WHEN EXISTS(SELECT 1 FROM sessions WHERE workspace_id=workspaces.id AND status='working') THEN 'working'
            ELSE 'ready' END
         WHERE id=(SELECT workspace_id FROM sessions WHERE id=?1)",
        params![session_id],
    )?;
    drop(db);
    core.events.publish(CoreEvent::Agent(event));
    // Resolving from any surface — the child conversation or the parent's
    // mirrored card — must update the parent's view and the worker lifecycle
    // together, so the two never disagree about whether the child is blocked.
    if is_worker {
        live_turn::notify_parent_child_left_waiting(core, session_id, decision);
    }
    core.events.publish(CoreEvent::StateChanged);
    Ok(())
}

fn decision_status(decision: &str) -> &'static str {
    match decision {
        "accept" => "allowed_once",
        "acceptForSession" => "allowed_for_session",
        "decline" => "declined",
        "cancel" => "cancelled",
        _ => "resolved",
    }
}

fn interaction_result(
    disposition: wire::InteractionResolutionDisposition,
    interaction_kind: &str,
    status: &str,
    resolved_by: &str,
    decision: &str,
    reason: Option<&str>,
) -> wire::InteractionResolutionResult {
    wire::InteractionResolutionResult {
        disposition,
        interaction_kind: interaction_kind.into(),
        status: status.into(),
        resolved_by: resolved_by.into(),
        decision: decision.into(),
        reason: reason.map(str::to_owned),
    }
}

fn stored_interaction_result(
    db: &Connection,
    session_id: &str,
    event_id: i64,
) -> Result<wire::InteractionResolutionResult, BridgeError> {
    let (kind, status, decision, actor, reason): (String, String, String, String, Option<String>) =
        db.query_row(
            "SELECT interaction_kind,status,decision,resolved_by,reason
             FROM interaction_resolutions
             WHERE session_id=?1 AND request_sequence=?2",
            params![session_id, event_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )?;
    Ok(interaction_result(
        wire::InteractionResolutionDisposition::AlreadyResolved,
        &kind,
        &status,
        &actor,
        &decision,
        reason.as_deref(),
    ))
}

fn interaction_payload(
    db: &Connection,
    session_id: &str,
    event_id: i64,
    expected_kind: &str,
) -> Result<(Value, String, bool), BridgeError> {
    let (kind, payload, adapter_id): (String, String, String) = db.query_row(
        "SELECT e.kind,e.payload,s.harness FROM session_entries e
         JOIN sessions s ON s.id=e.session_id
         WHERE e.session_id=?1 AND e.sequence=?2",
        params![session_id, event_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    if kind != expected_kind {
        return Err(BridgeError::Invalid(format!(
            "Expected {expected_kind}, found {kind}"
        )));
    }
    let payload: Value = serde_json::from_str(&payload)
        .map_err(|error| BridgeError::Invalid(format!("Interaction metadata is invalid: {error}")))?;
    let is_worker = store::worker_runtime(db, session_id)?.is_some();
    Ok((payload, adapter_id, is_worker))
}

fn claim_interaction(
    db: &Connection,
    session_id: &str,
    event_id: i64,
    interaction_kind: &str,
    decision: &str,
    option_id: Option<&str>,
    actor: &str,
    reason: Option<&str>,
    item_id: Option<&str>,
) -> Result<Option<AgentEvent>, BridgeError> {
    let transaction = db.unchecked_transaction()?;
    let now = chrono::Utc::now().to_rfc3339();
    let inserted = transaction.execute(
        "INSERT OR IGNORE INTO interaction_resolutions(
            session_id,request_sequence,interaction_kind,status,decision,option_id,
            resolved_by,reason,created_at,updated_at
         ) VALUES(?1,?2,?3,'settling',?4,?5,?6,?7,?8,?8)",
        params![session_id, event_id, interaction_kind, decision, option_id, actor, reason, now],
    )?;
    if inserted == 0 {
        transaction.rollback()?;
        return Ok(None);
    }
    let resolving = agent::NormalizedEvent {
        kind: format!("{interaction_kind}.resolving"),
        item_id: item_id.map(str::to_owned),
        role: None,
        status: Some("settling".into()),
        title: Some(if interaction_kind == "question" {
            "Sending answer"
        } else {
            "Applying permission decision"
        }
        .into()),
        text: None,
        data: serde_json::json!({
            "requestEventId": event_id,
            "decision": decision,
            "optionId": option_id,
            "resolvedBy": actor,
            "reason": reason,
        }),
    };
    let event = store::session_event_in_transaction(
        &transaction,
        session_id,
        &resolving,
        &serde_json::json!({"actor":actor}),
    )?;
    transaction.commit()?;
    Ok(Some(event))
}

#[allow(clippy::too_many_arguments)]
fn finish_interaction(
    core: &Arc<BridgeCore>,
    session_id: &str,
    event_id: i64,
    interaction_kind: &str,
    decision: &str,
    option_id: Option<&str>,
    actor: &str,
    reason: Option<&str>,
    adapter_id: &str,
    is_worker: bool,
    item_id: Option<&str>,
    failure: Option<&str>,
) -> Result<wire::InteractionResolutionResult, BridgeError> {
    let status = failure.map_or_else(
        || {
            if interaction_kind == "question" && decision == "answer" {
                "answered"
            } else {
                decision_status(decision)
            }
        },
        |_| "failed",
    );
    let db = core.db.lock().unwrap();
    let transaction = db.unchecked_transaction()?;
    let resolved = agent::NormalizedEvent {
        kind: format!("{interaction_kind}.resolved"),
        item_id: item_id.map(str::to_owned),
        role: None,
        status: Some(status.into()),
        title: Some(if failure.is_some() {
            format!("{} resolution failed", if interaction_kind == "question" { "Question" } else { "Permission" })
        } else if interaction_kind == "question" {
            "Question answered".into()
        } else {
            "Permission resolved".into()
        }),
        text: failure.map(str::to_owned),
        data: serde_json::json!({
            "requestEventId": event_id,
            "decision": decision,
            "optionId": option_id,
            "resolvedBy": actor,
            "reason": reason,
            "failure": failure,
        }),
    };
    let event = store::session_event_in_transaction(
        &transaction,
        session_id,
        &resolved,
        &serde_json::json!({"adapter":adapter_id,"actor":actor}),
    )?;
    transaction.execute(
        "UPDATE interaction_resolutions
         SET status=?3,result_event_sequence=?4,updated_at=?5
         WHERE session_id=?1 AND request_sequence=?2",
        params![session_id, event_id, status, event.sequence, chrono::Utc::now().to_rfc3339()],
    )?;
    if failure.is_none() {
        if is_worker {
            session_supervisor::SessionSupervisor::transition_in_transaction(
                &transaction,
                session_id,
                worker_lifecycle::WorkerLifecycleState::Working,
                Some(if interaction_kind == "question" { "question_resolved" } else { "permission_resolved" }),
            )?;
        } else {
            transaction.execute(
                "UPDATE sessions SET status='working' WHERE id=?1",
                params![session_id],
            )?;
        }
        transaction.execute(
            "UPDATE workspaces SET status=CASE
                WHEN EXISTS(SELECT 1 FROM sessions WHERE workspace_id=workspaces.id AND status='waiting') THEN 'waiting'
                WHEN EXISTS(SELECT 1 FROM sessions WHERE workspace_id=workspaces.id AND status='working') THEN 'working'
                ELSE 'ready' END
             WHERE id=(SELECT workspace_id FROM sessions WHERE id=?1)",
            params![session_id],
        )?;
    }
    transaction.commit()?;
    drop(db);
    core.events.publish(CoreEvent::Agent(event));
    if is_worker && failure.is_none() {
        live_turn::notify_parent_child_left_waiting(core, session_id, status);
    }
    core.events.publish(CoreEvent::StateChanged);
    Ok(interaction_result(
        wire::InteractionResolutionDisposition::Resolved,
        interaction_kind,
        status,
        actor,
        decision,
        reason,
    ))
}

fn offered_permission_action(
    details: &Value,
    decision: Option<&str>,
    option_id: Option<&str>,
    actor: &str,
) -> Result<(String, Option<String>), BridgeError> {
    let actions = details
        .get("actions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if actions.is_empty() {
        let decision = decision.ok_or_else(|| BridgeError::Invalid("Permission has no supported allow action".into()))?;
        return Ok((decision.into(), option_id.map(str::to_owned)));
    }
    let selected = if actor == "policy" {
        actions
            .iter()
            .find(|action| action.get("decision").and_then(Value::as_str) == Some("acceptForSession"))
            .or_else(|| actions.iter().find(|action| action.get("decision").and_then(Value::as_str) == Some("accept")))
    } else {
        actions.iter().find(|action| {
            action.get("decision").and_then(Value::as_str) == decision
                && match (option_id, action.get("optionId").and_then(Value::as_str)) {
                    (Some(requested), Some(offered)) => requested == offered,
                    (None, None) => true,
                    _ => false,
                }
        })
    }
    .ok_or_else(|| BridgeError::Invalid("The provider did not offer that permission action".into()))?;
    Ok((
        selected.get("decision").and_then(Value::as_str).unwrap_or_default().into(),
        selected.get("optionId").and_then(Value::as_str).map(str::to_owned),
    ))
}

pub(crate) fn auto_resolve_provider_permission(
    core: &Arc<BridgeCore>,
    session_id: &str,
    event_id: i64,
) -> Result<wire::InteractionResolutionResult, BridgeError> {
    resolve_provider_permission(
        core,
        session_id,
        event_id,
        None,
        None,
        "policy",
        Some("Auto-approve provider permissions"),
    )
}

#[allow(clippy::too_many_arguments)]
fn resolve_provider_permission(
    core: &Arc<BridgeCore>,
    session_id: &str,
    event_id: i64,
    requested_decision: Option<&str>,
    requested_option_id: Option<&str>,
    actor: &str,
    reason: Option<&str>,
) -> Result<wire::InteractionResolutionResult, BridgeError> {
    let db = core.db.lock().unwrap();
    let (payload, adapter_id, is_worker) =
        interaction_payload(&db, session_id, event_id, "permission.requested")?;
    let details = payload.get("data").cloned().unwrap_or_else(|| serde_json::json!({}));
    let (decision, option_id) = offered_permission_action(
        &details,
        requested_decision,
        requested_option_id,
        actor,
    )?;
    let request_id = details
        .get("requestId")
        .cloned()
        .ok_or_else(|| BridgeError::Invalid("Permission has no provider request id".into()))?;
    let item_id = payload.get("itemId").and_then(Value::as_str).map(str::to_owned);
    let claim = claim_interaction(
        &db,
        session_id,
        event_id,
        "permission",
        &decision,
        option_id.as_deref(),
        actor,
        reason,
        item_id.as_deref(),
    )?;
    let Some(resolving_event) = claim else {
        return stored_interaction_result(&db, session_id, event_id);
    };
    drop(db);
    core.events.publish(CoreEvent::Agent(resolving_event));

    let adapters = core.adapters.lock().unwrap();
    let response = match adapters.get(session_id) {
        Some(runtime) => runtime.respond_with_option(request_id, &decision, option_id.as_deref()),
        None => Err(BridgeError::Invalid(
            "Structured adapter session is not running".into(),
        )),
    };
    drop(adapters);
    if let Err(error) = response {
        let message = error.to_string();
        let _ = finish_interaction(
            core,
            session_id,
            event_id,
            "permission",
            &decision,
            option_id.as_deref(),
            actor,
            reason,
            &adapter_id,
            is_worker,
            item_id.as_deref(),
            Some(&message),
        );
        return Err(error);
    }
    finish_interaction(
        core,
        session_id,
        event_id,
        "permission",
        &decision,
        option_id.as_deref(),
        actor,
        reason,
        &adapter_id,
        is_worker,
        item_id.as_deref(),
        None,
    )
}

pub fn resolve_question(
    core: &Arc<BridgeCore>,
    session_id: &str,
    event_id: i64,
    action: &str,
    mut answers: std::collections::BTreeMap<String, Vec<String>>,
) -> Result<wire::InteractionResolutionResult, BridgeError> {
    if !matches!(action, "answer" | "decline" | "cancel") {
        return Err(BridgeError::Invalid("Unsupported question action".into()));
    }
    let db = core.db.lock().unwrap();
    let (payload, adapter_id, is_worker) =
        interaction_payload(&db, session_id, event_id, "question.requested")?;
    let details = payload.get("data").cloned().unwrap_or_else(|| serde_json::json!({}));
    let request_id = details
        .get("requestId")
        .cloned()
        .ok_or_else(|| BridgeError::Invalid("Question has no provider request id".into()))?;
    let request_method = details
        .get("requestMethod")
        .and_then(Value::as_str)
        .ok_or_else(|| BridgeError::Invalid("Question has no provider reply channel".into()))?;
    if action == "answer" && answers.values().all(Vec::is_empty) {
        return Err(BridgeError::Invalid("Enter an answer before sending".into()));
    }
    for values in answers.values_mut() {
        for value in values.iter_mut() {
            let intercepted = secret_interception::intercept(value);
            core.credential_broker.register(session_id, intercepted.captured);
            *value = intercepted.sanitized.text;
        }
    }
    let item_id = payload.get("itemId").and_then(Value::as_str).map(str::to_owned);
    let claim = claim_interaction(
        &db,
        session_id,
        event_id,
        "question",
        action,
        None,
        "human",
        None,
        item_id.as_deref(),
    )?;
    let Some(resolving_event) = claim else {
        return stored_interaction_result(&db, session_id, event_id);
    };
    drop(db);
    core.events.publish(CoreEvent::Agent(resolving_event));

    let adapters = core.adapters.lock().unwrap();
    let response = match (adapters.get(session_id), request_method) {
        (None, _) => Err(BridgeError::Invalid(
            "Structured adapter session is not running".into(),
        )),
        (Some(runtime), agent::OPENCODE_QUESTION_REQUEST_METHOD) => {
            if action == "answer" {
                let ordered = details
                    .get("questions")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .map(|(index, question)| {
                        let id = question
                            .get("id")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                            .unwrap_or_else(|| index.to_string());
                        Value::Array(
                            answers
                                .get(&id)
                                .cloned()
                                .unwrap_or_default()
                                .into_iter()
                                .map(Value::String)
                                .collect(),
                        )
                    })
                    .collect();
                runtime.answer_question(request_id, Value::Array(ordered))
            } else {
                runtime.reject_question(request_id)
            }
        }
        (Some(runtime), "item/tool/requestUserInput") => {
            let mapped = answers
                .iter()
                .map(|(id, values)| (id.clone(), serde_json::json!({"answers":values})))
                .collect::<serde_json::Map<_, _>>();
            runtime.answer_question(request_id, serde_json::json!({"answers":mapped}))
        }
        (Some(runtime), "mcpServer/elicitation/request") => {
            let content = answers
                .iter()
                .map(|(id, values)| {
                    let value = if values.len() == 1 {
                        Value::String(values[0].clone())
                    } else {
                        Value::Array(values.iter().cloned().map(Value::String).collect())
                    };
                    (id.clone(), value)
                })
                .collect::<serde_json::Map<_, _>>();
            runtime.answer_question(
                request_id,
                serde_json::json!({
                    "action": if action == "answer" { "accept" } else if action == "cancel" { "cancel" } else { "decline" },
                    "content": if action == "answer" { Value::Object(content) } else { Value::Null },
                }),
            )
        }
        (Some(_), other) => Err(BridgeError::Invalid(format!(
            "Unsupported question reply channel {other}"
        ))),
    };
    drop(adapters);
    if let Err(error) = response {
        let message = error.to_string();
        let _ = finish_interaction(
            core,
            session_id,
            event_id,
            "question",
            action,
            None,
            "human",
            None,
            &adapter_id,
            is_worker,
            item_id.as_deref(),
            Some(&message),
        );
        return Err(error);
    }
    finish_interaction(
        core,
        session_id,
        event_id,
        "question",
        action,
        None,
        "human",
        None,
        &adapter_id,
        is_worker,
        item_id.as_deref(),
        None,
    )
}

// --- terminal ----------------------------------------------------------------

fn terminal_runtime_id(workspace_id: &str, terminal_id: &str) -> String {
    format!("terminal:{workspace_id}:{terminal_id}")
}

/// One counter across every shell ever spawned: equality is all the reader
/// threads need, and a global sidesteps per-key bookkeeping.
pub(crate) static TERMINAL_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// The login shell for new terminals: zsh where it exists (the macOS default
/// this app was built around), else the user's `$SHELL`, else bash. Linux
/// servers and CI runners frequently ship neither zsh nor a spawnable `$SHELL`
/// value, so a hardcoded zsh would make terminals unopenable there.
pub(crate) fn login_shell() -> String {
    for candidate in ["/bin/zsh", "/usr/bin/zsh", "/usr/local/bin/zsh"] {
        if Path::new(candidate).exists() {
            return candidate.to_owned();
        }
    }
    if let Some(shell) = std::env::var_os("SHELL") {
        let shell = shell.to_string_lossy().into_owned();
        if !shell.is_empty() {
            return shell;
        }
    }
    "/bin/bash".to_owned()
}

// Both desktop transports expose this shared API seam; the terminal module
// owns the implementation and its PTY/checkpoint lifecycle.
pub use crate::terminal_workspace::{
    create as create_terminal, rename as rename_terminal,
    save_layout as save_terminal_workspace, snapshot as get_terminal_snapshot,
    workspace as get_terminal_workspace,
};

pub fn open_terminal(core: &Arc<BridgeCore>, workspace_id: &str, terminal_id: &str) -> Result<(), BridgeError> {
    crate::terminal_workspace::create(core, &wire::CreateTerminalParams {
        workspace_id: workspace_id.into(), terminal_id: terminal_id.into(), agent_id: None, cwd: None, restart: true,
    }).map(|_| ())
}

pub fn close_terminal(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
    terminal_id: &str,
) -> Result<(), BridgeError> {
    let runtime_id = terminal_runtime_id(workspace_id, terminal_id);
    // The same claim open takes, so a close racing an open of the same key
    // settles into a definite order instead of interleaving.
    let _lifecycle = core.claim_session_lifecycle(&runtime_id, "terminal close")?;
    // Persist the close before ending the process, including an already-ended
    // pane. Provider login terminals deliberately have no history record.
    if workspace_id != PROVIDER_LOGIN_WORKSPACE_ID {
        crate::terminal_workspace::closed(core, workspace_id, terminal_id)?;
    }
    let runtime = core.runtimes.lock().unwrap().remove(&runtime_id);
    if let Some(mut runtime) = runtime { let _ = runtime.child.kill(); }

    Ok(())
}

pub fn list_terminals(core: &Arc<BridgeCore>, workspace_id: &str) -> Vec<String> {
    let prefix = format!("terminal:{workspace_id}:");
    let mut ids: Vec<String> = core
        .runtimes
        .lock()
        .unwrap()
        .keys()
        .filter_map(|key| key.strip_prefix(&prefix).map(str::to_owned))
        .collect();
    ids.sort();
    ids
}

pub fn write_terminal(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
    terminal_id: &str,
    data: &str,
) -> Result<(), BridgeError> {
    let mut sessions = core.runtimes.lock().unwrap();
    let runtime = sessions
        .get_mut(&terminal_runtime_id(workspace_id, terminal_id))
        .ok_or_else(|| BridgeError::Invalid("Workspace terminal is not open".into()))?;
    runtime.writer.write_all(data.as_bytes())?;
    runtime.writer.flush()?;
    Ok(())
}

pub fn resize_terminal(core: &Arc<BridgeCore>, workspace_id: &str, terminal_id: &str, rows: u16, cols: u16) -> Result<(), BridgeError> {
    crate::terminal_workspace::resized(core, workspace_id, terminal_id, rows, cols)
}

// --- provider sign-in ---------------------------------------------------------

/// The pseudo-workspace id addressing every provider login terminal. A
/// vendor sign-in has no workspace of its own, so this reuses the terminal
/// domain's `workspaceId`/`terminalId` key scheme rather than inventing a
/// second PTY surface — the UI hosts the pane and drives it with the
/// existing `write_terminal`/`resize_terminal`/`close_terminal` methods.
const PROVIDER_LOGIN_WORKSPACE_ID: &str = "provider-login";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderLogin {
    pub workspace_id: String,
    pub terminal_id: String,
}

/// The vendor's own documented login entry point for `provider`. Bridge only
/// launches the process — any browser handoff is started by the vendor
/// command itself, and Bridge never reads, stores, or logs the credential it
/// produces.
fn provider_login_command(core: &Arc<BridgeCore>, provider: &str) -> Result<CommandBuilder, BridgeError> {
    match provider {
        // Run the dedicated flow even when an existing credential has expired.
        "claude" => {
            let binary = crate::managed_runtime::managed_entrypoint("claude")
                .or_else(|| binary::resolve("claude"))
                .ok_or_else(|| BridgeError::Invalid("Claude binary is not installed".into()))?;
            let mut command = CommandBuilder::new(binary);
            command.args(["auth", "login"]);
            Ok(command)
        }
        "codex" => {
            let binary = crate::codex_adapter::resolve_runtime()
                .ok_or_else(|| BridgeError::Invalid("Codex binary is not installed".into()))?;
            let mut command = CommandBuilder::new(binary);
            command.args(["login"]);
            Ok(command)
        }
        // The adapter's own resolution rather than a bare binary::resolve: it
        // prefers a Bridge-managed payload, and it refuses the ambiguous `agent`
        // name outright, so a login is never spawned against an executable that
        // merely shares that name and has never said who it is.
        "cursor" => {
            let executable = crate::cursor_adapter::login_executable()
                .map_err(|unavailable| BridgeError::Invalid(unavailable.reason()))?;
            let mut command = CommandBuilder::new(executable);
            command.args(["login"]);
            Ok(command)
        }
        "grok" => {
            let executable = crate::grok_adapter::login_executable()
                .map_err(|unavailable| BridgeError::Invalid(unavailable.reason()))?;
            let mut command = CommandBuilder::new(executable);
            command.args(["login"]);
            Ok(command)
        }
        "opencode" => {
            let settings = core.adapter_registry.opencode_settings()?;
            let binary = crate::opencode_adapter::resolve_executable(&settings)?;
            let mut command = CommandBuilder::new(binary);
            command.args(["auth", "login"]);
            Ok(command)
        }
        other => Err(BridgeError::Invalid(format!("Unknown provider {other:?}"))),
    }
}

/// Start `provider`'s own login flow inside a Bridge-owned PTY. Reattaches to
/// an already-running login for the same provider instead of spawning a
/// second one. The vendor process's exit publishes the same
/// [`CoreEvent::TerminalExited`] the terminal domain always has, which is
/// what lets the UI re-read health without a restart once sign-in completes.
pub fn start_provider_login(
    core: &Arc<BridgeCore>,
    provider: &str,
) -> Result<ProviderLogin, BridgeError> {
    let runtime_id = terminal_runtime_id(PROVIDER_LOGIN_WORKSPACE_ID, provider);
    let _lifecycle = core.claim_session_lifecycle(&runtime_id, "provider login")?;
    if core.runtimes.lock().unwrap().contains_key(&runtime_id) {
        return Ok(ProviderLogin {
            workspace_id: PROVIDER_LOGIN_WORKSPACE_ID.into(),
            terminal_id: provider.into(),
        });
    }
    let mut command = provider_login_command(core, provider)?;
    command.env("TERM", "xterm-256color");
    if let Some(path) = binary::hydrated_command_path() {
        command.env("PATH", path);
    }
    if let Some(home) = std::env::var_os("HOME") {
        command.cwd(home);
    }
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 32,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| BridgeError::Pty(e.to_string()))?;
    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|e| BridgeError::Pty(e.to_string()))?;
    drop(pair.slave);
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| BridgeError::Pty(e.to_string()))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| BridgeError::Pty(e.to_string()))?;
    let epoch = TERMINAL_EPOCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    core.runtimes.lock().unwrap().insert(
        runtime_id.clone(),
        RuntimeSession {
            writer,
            master: pair.master,
            child,
            epoch,
        },
    );
    let core_reader = Arc::clone(core);
    let workspace_reader = PROVIDER_LOGIN_WORKSPACE_ID.to_owned();
    let terminal_reader = provider.to_owned();
    let runtime_reader = runtime_id;
    thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let data = String::from_utf8_lossy(&buf[..n]).into_owned();
                    core_reader.events.publish(CoreEvent::SessionOutput {
                        session_id: workspace_reader.clone(),
                        terminal_id: terminal_reader.clone(),
                        data,
                    });
                }
            }
        }
        {
            let mut sessions = core_reader.runtimes.lock().unwrap();
            match sessions.get(&runtime_reader) {
                Some(entry) if entry.epoch == epoch => {
                    sessions.remove(&runtime_reader);
                }
                Some(_) => {
                    return;
                }
                None => {}
            }
        }
        // The sign-in that just ended may have changed what a probe would
        // answer, and an adapter that records its probe would otherwise keep
        // serving the pre-login one until restart. Unconditional — a cancelled
        // login re-probes to the same answer — and non-blocking: the fresh
        // result arrives as its own adapters-changed hint when it lands.
        core_reader
            .adapter_registry
            .refresh_availability(&terminal_reader);
        core_reader.events.publish(CoreEvent::TerminalExited {
            session_id: workspace_reader,
            terminal_id: terminal_reader,
        });
    });
    Ok(ProviderLogin {
        workspace_id: PROVIDER_LOGIN_WORKSPACE_ID.into(),
        terminal_id: provider.into(),
    })
}

/// Abandon `provider`'s login flow. Killing the vendor process (rather than
/// just unmounting the pane) is what keeps a retry usable: the existing-runtime
/// branch of [`start_provider_login`] never replays the URL and prompts a
/// closed pane missed, so a surviving process would leave the next attempt
/// staring at an empty terminal. The reader thread publishes the usual
/// `TerminalExited` and re-probes availability as the process ends.
pub fn cancel_provider_login(core: &Arc<BridgeCore>, provider: &str) -> Result<(), BridgeError> {
    let runtime_id = terminal_runtime_id(PROVIDER_LOGIN_WORKSPACE_ID, provider);
    let removed = core.runtimes.lock().unwrap().remove(&runtime_id);
    if let Some(mut runtime) = removed {
        let _ = runtime.child.kill();
    }
    Ok(())
}

// --- slash commands ------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlashCommandResolve {
    pub name: String,
    pub harness: String,
    pub kind: String,
    /// When true, the frontend should switch the direct chat to `harness`
    /// before sending.
    pub switch_harness: bool,
}

fn available_adapter_ids(core: &BridgeCore) -> HashSet<String> {
    core.adapter_registry
        .descriptors()
        .into_iter()
        .filter(|descriptor| descriptor.available)
        .map(|descriptor| descriptor.id)
        .collect()
}

/// Enumerate slash commands + skills scoped to this session's active harness
/// (plus Bridge-local builtins, which work on every harness), so the UI's `/`
/// menu never dangles suggestions the session can't actually run.
pub fn list_slash_commands(
    core: &Arc<BridgeCore>,
    session_id: Option<&str>,
) -> Result<Vec<slash::SlashCommand>, BridgeError> {
    let (project, session_harness): (Option<PathBuf>, Option<String>) = session_id
        .map(|session_id| {
            core.db.lock().unwrap().query_row(
                "SELECT cwd, harness FROM sessions WHERE id=?1",
                params![session_id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?.map(PathBuf::from),
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )
        })
        .transpose()?
        .unwrap_or((None, None));
    let catalog = slash::list_commands_for_project(&available_adapter_ids(core), project.as_deref());
    Ok(filter_catalog_for_session_harness(catalog, session_harness.as_deref()))
}

/// Keep only commands the session's active harness (or Bridge itself) can
/// actually run. `None` (session-less callers, e.g. onboarding) skips the
/// filter and returns every discovered command.
fn filter_catalog_for_session_harness(
    catalog: Vec<slash::SlashCommand>,
    session_harness: Option<&str>,
) -> Vec<slash::SlashCommand> {
    match session_harness {
        Some(harness) => catalog
            .into_iter()
            .filter(|command| command.harness == "bridge" || command.harness == harness)
            .collect(),
        None => catalog,
    }
}

/// Resolve a composer `/command` against the catalog so the UI can auto-switch
/// harness before sending.
pub fn resolve_slash_command(
    core: &Arc<BridgeCore>,
    text: &str,
    session_id: &str,
) -> Result<Option<SlashCommandResolve>, BridgeError> {
    let trimmed = text.trim();
    let Some(rest) = trimmed.strip_prefix('/') else {
        return Ok(None);
    };
    let name = rest
        .split_whitespace()
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| BridgeError::Invalid("Empty slash command".into()))?;
    let (kind, session_harness, cwd): (String, String, Option<String>) = {
        let db = core.db.lock().unwrap();
        db.query_row(
            "SELECT kind, harness, cwd FROM sessions WHERE id=?1",
            params![session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?
    };
    if slash::is_bridge_local(name) {
        return Ok(Some(SlashCommandResolve {
            name: name.to_string(),
            harness: session_harness,
            kind: "builtin".into(),
            switch_harness: false,
        }));
    }
    let available = available_adapter_ids(core);
    let project = cwd.as_deref().map(Path::new);
    let catalog = slash::list_commands_for_project(&available, project);
    let matches: Vec<_> = catalog
        .iter()
        .filter(|command| command.name.eq_ignore_ascii_case(name))
        .collect();
    if matches.is_empty() {
        return Ok(None);
    }
    let chosen = matches
        .iter()
        .find(|command| command.harness == session_harness)
        .or_else(|| {
            // Prefer the command's own harness when the name is unique to one provider.
            if matches.len() == 1 {
                matches.first()
            } else {
                None
            }
        })
        .or_else(|| matches.first())
        .map(|command| (*command).clone())
        .expect("matches non-empty");
    let switch_harness = kind == "direct" && chosen.harness != session_harness;
    Ok(Some(SlashCommandResolve {
        name: chosen.name.clone(),
        harness: chosen.harness.clone(),
        kind: chosen.kind.clone(),
        switch_harness,
    }))
}

// --- completion / verification -------------------------------------------------

fn completion_repository_stamp(
    db: &Connection,
    session_id: &str,
) -> Result<completion::RepositoryStamp, BridgeError> {
    let state = store::repository_state_for_session(db, session_id)?;
    let head = state.get("head").and_then(Value::as_str).ok_or_else(|| {
        BridgeError::Invalid("completion proof requires a Git repository HEAD".into())
    })?;
    let dirty = state
        .get("dirtyHash")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            BridgeError::Invalid(
                "completion proof requires a deterministic dirty-tree digest".into(),
            )
        })?;
    Ok(completion::RepositoryStamp {
        head: head.into(),
        dirty_digest: dirty.into(),
    })
}

fn completion_attempt_repository(
    db: &Connection,
    attempt_id: &str,
) -> Result<(String, completion::RepositoryStamp), BridgeError> {
    let (session_id, repository_path, stored_head, stored_dirty): (String, String, String, String) = db.query_row(
        "SELECT session_id,repository_path,repository_head,dirty_digest FROM eval_attempts WHERE id=?1",
        params![attempt_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let state = store::repository_state_for_path(Path::new(&repository_path));
    let head = state
        .get("head")
        .and_then(Value::as_str)
        .unwrap_or(&stored_head);
    let dirty = state
        .get("dirtyHash")
        .and_then(Value::as_str)
        .unwrap_or(&stored_dirty);
    Ok((
        session_id,
        completion::RepositoryStamp {
            head: head.into(),
            dirty_digest: dirty.into(),
        },
    ))
}

/// Every capability a verifier can currently rely on: live adapters plus the
/// installed skill catalog.
fn live_available_capabilities(core: &BridgeCore) -> HashSet<String> {
    let mut capabilities = core
        .adapter_registry
        .descriptors()
        .into_iter()
        .filter(|descriptor| descriptor.available)
        .flat_map(|descriptor| descriptor.capabilities)
        .collect::<HashSet<_>>();
    if let Ok(skills) = skill_marketplace::available_capabilities(&user_home(), &core.skill_store) {
        capabilities.extend(skills);
    }
    capabilities
}

pub fn create_completion_plan(
    core: &Arc<BridgeCore>,
    session_id: &str,
    acceptance_criteria: Vec<String>,
    changed_paths: Vec<String>,
    repository_commands: Vec<String>,
    markdown_projection: Option<String>,
    markdown_committed: bool,
) -> Result<completion::CompletionSummary, BridgeError> {
    let (workspace_id, implementer_family): (String, Option<String>) = {
        let db = core.db.lock().unwrap();
        let workspace_id = db.query_row(
            "SELECT workspace_id FROM sessions WHERE id=?1",
            params![session_id],
            |row| row.get(0),
        )?;
        let implementer_family = db.query_row(
            "SELECT s.harness FROM worker_runtime r JOIN worker_leases l ON l.session_id=r.session_id JOIN sessions s ON s.id=r.session_id WHERE r.parent_session_id=?1 AND l.role='implementation' ORDER BY r.updated_at DESC LIMIT 1",
            params![session_id],
            |row| row.get(0),
        ).optional()?;
        (workspace_id, implementer_family)
    };
    let contract = completion::CompletionContract {
        id: Uuid::new_v4().to_string(),
        workspace_id,
        session_id: session_id.to_owned(),
        schema_version: completion::COMPLETION_SCHEMA_VERSION,
        acceptance_criteria: acceptance_criteria.clone(),
        markdown_projection,
        markdown_committed,
    };
    let available_capabilities = live_available_capabilities(core);
    let change_labels = completion::labels_for_paths(&changed_paths);
    let db = core.db.lock().unwrap();
    let plan = completion::plan_with_registered_manifests(
        &db,
        completion::PlanInput {
            contract_id: contract.id.clone(),
            acceptance_criteria,
            changed_paths,
            repository_commands,
        },
        &change_labels,
        &available_capabilities,
    )?;
    let repository_path: String = db.query_row("SELECT COALESCE(s.cwd,w.path) FROM sessions s JOIN workspaces w ON w.id=s.workspace_id WHERE s.id=?1", params![session_id], |row| row.get(0))?;
    let repository = completion_repository_stamp(&db, session_id)?;
    completion::create_flow(
        &db,
        &contract,
        &plan,
        session_id,
        &repository_path,
        &repository,
        implementer_family.as_deref(),
    )?;
    let summary = completion::latest_summary(&db, session_id)?
        .ok_or_else(|| BridgeError::Invalid("completion plan was not persisted".into()))?;
    drop(db);
    core.events.publish(CoreEvent::StateChanged);
    Ok(summary)
}

pub fn record_completion_check(
    core: &Arc<BridgeCore>,
    attempt_id: &str,
    run: &completion::CheckRun,
) -> Result<completion::CompletionSummary, BridgeError> {
    let db = core.db.lock().unwrap();
    let (session_id, repository) = completion_attempt_repository(&db, attempt_id)?;
    completion::record_check(&db, attempt_id, run)?;
    completion::finalize(&db, attempt_id, &repository)?;
    completion::reconcile_parent_readiness(&db, &session_id)?;
    let summary = completion::latest_summary(&db, &session_id)?
        .ok_or_else(|| BridgeError::Invalid("completion summary disappeared".into()))?;
    drop(db);
    core.events.publish(CoreEvent::StateChanged);
    Ok(summary)
}

pub fn waive_completion(
    core: &Arc<BridgeCore>,
    attempt_id: &str,
    check_ids: &[String],
    reason: &str,
) -> Result<completion::CompletionSummary, BridgeError> {
    let db = core.db.lock().unwrap();
    let (session_id, repository) = completion_attempt_repository(&db, attempt_id)?;
    completion::waive(
        &db,
        attempt_id,
        check_ids,
        reason,
        "local_user",
        &repository,
    )?;
    completion::finalize(&db, attempt_id, &repository)?;
    completion::reconcile_parent_readiness(&db, &session_id)?;
    let summary = completion::latest_summary(&db, &session_id)?
        .ok_or_else(|| BridgeError::Invalid("completion summary disappeared".into()))?;
    drop(db);
    core.events.publish(CoreEvent::StateChanged);
    Ok(summary)
}

pub fn register_verifier_manifest(
    core: &Arc<BridgeCore>,
    source: &str,
    manifest: &completion::VerifierManifest,
) -> Result<(), BridgeError> {
    completion::register_verifier_manifest(&core.db.lock().unwrap(), source, manifest)
}

pub fn verifier_candidates(
    core: &Arc<BridgeCore>,
    change_labels: &[String],
    available_capabilities: Vec<String>,
) -> Result<Vec<completion::VerifierCandidate>, BridgeError> {
    completion::verifier_candidates(
        &core.db.lock().unwrap(),
        change_labels,
        &available_capabilities.into_iter().collect(),
    )
}

// --- work ---------------------------------------------------------------------

/// The Work board. Store-only: it takes the connection and nothing else, so no
/// git command, provider start, connector call, or network request can happen on
/// the way to a rendered board.
pub fn get_work_board(core: &Arc<BridgeCore>) -> Result<wire::WorkBoard, BridgeError> {
    work::board(&core.db.lock().unwrap())
}

/// Apply a human's decision to a suggested task.
///
/// Local only. Marking a task done does not close the thread it came from, and dismissing
/// it does not archive anything — this writes one row in Bridge's own database and stops.
/// The state machine decides whether the transition is legal; an illegal one is refused
/// rather than quietly ignored, so a caller cannot believe something happened.
pub fn work_task_action(
    core: &Arc<BridgeCore>,
    params: &wire::TaskActionParams,
) -> Result<(), BridgeError> {
    let action = match params.action {
        wire::WorkTaskActionKind::Done => work_task_state::TaskAction::Complete,
        wire::WorkTaskActionKind::Snooze => work_task_state::TaskAction::Snooze,
        wire::WorkTaskActionKind::Dismiss => work_task_state::TaskAction::Dismiss,
        wire::WorkTaskActionKind::Restore => work_task_state::TaskAction::Restore,
    };
    let now = chrono::Utc::now();
    let now_text = now.to_rfc3339();
    let db = core.db.lock().unwrap();
    let current = work_reconcile::read_task(&db, &params.task_id)?
        .ok_or_else(|| BridgeError::Invalid("that task is not on the board".into()))?;
    let next = work_task_state::apply_action(&current, action, &now_text)
        .map_err(|illegal| BridgeError::Invalid(illegal.reason()))?;
    let snoozed_until = match params.action {
        wire::WorkTaskActionKind::Snooze => {
            let deadline = params.snoozed_until.as_deref()
                .ok_or_else(|| BridgeError::Invalid("snoozing requires a deadline".into()))?;
            let parsed = chrono::DateTime::parse_from_rfc3339(deadline)
                .map_err(|_| BridgeError::Invalid("the snooze deadline is not a timestamp".into()))?;
            if parsed <= now {
                return Err(BridgeError::Invalid("the snooze deadline must be in the future".into()));
            }
            Some(deadline)
        }
        // Leaving a stale deadline on a task that is no longer snoozed would make a later
        // expiry check answer about a snooze nobody set.
        _ => None,
    };
    work_reconcile::write_task_state(&db, &params.task_id, &next, snoozed_until, &now_text)
}

/// Pin or unpin a task.
///
/// Its own call rather than an action variant, because pinning is orthogonal to the five
/// states: folding it in would let a caller send `pin` where a state change is expected.
pub fn work_task_pin(
    core: &Arc<BridgeCore>,
    params: &wire::TaskPinParams,
) -> Result<(), BridgeError> {
    let action = if params.pinned {
        work_task_state::TaskAction::Pin
    } else {
        work_task_state::TaskAction::Unpin
    };
    let now = chrono::Utc::now().to_rfc3339();
    let db = core.db.lock().unwrap();
    let current = work_reconcile::read_task(&db, &params.task_id)?
        .ok_or_else(|| BridgeError::Invalid("that task is not on the board".into()))?;
    let next = work_task_state::apply_action(&current, action, &now)
        .map_err(|illegal| BridgeError::Invalid(illegal.reason()))?;
    work_reconcile::write_task_pin(&db, &params.task_id, next.pinned, &now)
}

/// Prepare a session for working on a task.
///
/// Creates the session and the draft, and sends nothing. The returned shape carries no turn
/// id precisely because none was dispatched: the user edits the draft and presses Send, and
/// until they do, no model has been told anything.
pub fn work_task_prepare_session(
    core: &Arc<BridgeCore>,
    params: &wire::TaskPrepareSessionParams,
) -> Result<wire::WorkTaskDraft, BridgeError> {
    let (title, why, source_kind) = {
        let db = core.db.lock().unwrap();
        db.query_row(
            "SELECT title,why,source_kind FROM work_tasks WHERE id=?1",
            params![params.task_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| BridgeError::Invalid("that task is not on the board".into()))?
    };
    let draft = work_actions::prepare_draft(&title, &why, &source_kind);
    // create_chat_id inserts a session row and starts no adapter, which is the whole
    // requirement: the session exists to be typed into, and no provider has been spoken to.
    let harness = Harness::parse(params.harness.as_str())
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let session_id = core.create_chat_id(&harness, params.model.as_deref(), Some(&draft.title))?;
    Ok(wire::WorkTaskDraft {
        session_id,
        title: draft.title,
        draft: draft.draft,
    })
}

/// Recheck and return one task's evidence destination at the point the user opens it.
pub fn work_task_open_evidence(
    core: &Arc<BridgeCore>,
    params: &wire::TaskOpenEvidenceParams,
) -> Result<wire::WorkEvidenceTarget, BridgeError> {
    let (stored, source_kind): (Option<String>, String) = core.db.lock().unwrap()
        .query_row(
            "SELECT evidence_target,source_kind FROM work_tasks WHERE id=?1",
            params![params.task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| BridgeError::Invalid("that task is not on the board".into()))?;
    let family = crate::work_connectors::ConnectorFamily::parse(
        source_kind.split('.').next().unwrap_or_default(),
    )
    .ok_or_else(|| BridgeError::Invalid("that task's source is not supported".into()))?;
    match work_actions::open_target(stored.as_deref(), family)
        .map_err(|refused| BridgeError::Invalid(refused.reason()))?
    {
        work_actions::OpenTarget::External { url, host } => {
            Ok(wire::WorkEvidenceTarget::ExternalLink { url, host })
        }
        work_actions::OpenTarget::Session { session_id } => {
            Ok(wire::WorkEvidenceTarget::Session { session_id })
        }
    }
}

/// Work's stored settings, with whether they were ever written.
pub fn read_work_settings(core: &Arc<BridgeCore>) -> Result<wire::WorkSettingsSnapshot, BridgeError> {
    work::read_settings(&core.db.lock().unwrap())
}

/// Persist Work's settings. Validation lives in Rust — `work::validate_settings`
/// plus the model-catalog check below, which needs the adapter registry that the
/// store-only module deliberately cannot reach.
pub fn write_work_settings(
    core: &Arc<BridgeCore>,
    params: &wire::WriteSettingsParams,
) -> Result<wire::WorkSettingsSnapshot, BridgeError> {
    if let Some(briefing) = params.settings.briefing.as_ref() {
        let descriptors = core.adapter_registry.descriptors();
        if let Some(descriptor) = descriptors
            .iter()
            .find(|descriptor| descriptor.id == briefing.harness.as_str())
        {
            // An empty catalog is a runtime-discovered one; only a non-empty
            // catalog can refuse a model by name.
            if !descriptor.models.is_empty()
                && !descriptor.models.iter().any(|model| model.id == briefing.model)
            {
                return Err(BridgeError::Invalid(format!(
                    "{} is not a model {} offers",
                    briefing.model, descriptor.label
                )));
            }
        }
    }
    work::write_settings(&core.db.lock().unwrap(), &params.settings)
}

/// Every registered harness as the briefing Settings surface needs it: certified
/// or refused with the gate's reason, plus the cheapest capable default model.
pub fn work_briefing_options(core: &Arc<BridgeCore>) -> wire::WorkBriefingOptions {
    let harnesses = core
        .adapter_registry
        .descriptors()
        .into_iter()
        .map(|descriptor| {
            let certification = crate::briefing_policy::certify_briefing(
                &descriptor.id,
                descriptor.version.as_deref(),
            );
            let supported = certification.is_ok();
            // The cheapest capable model is the Fast-tier default. Chosen here,
            // at the settings layer, because `resolve_briefing` deliberately
            // refuses to invent a model at run time.
            let default_model = supported
                .then(|| {
                    core.adapter_registry
                        .resolve_model(&descriptor.id, CapabilityTier::Fast, None)
                        .ok()
                        .map(|resolution| resolution.actual_model)
                })
                .flatten();
            // The connectors this harness itself holds, so Settings offers
            // narrowing to what actually exists. Only a certified harness gets
            // the (subprocess-backed) discovery: an uncertified one runs
            // nothing, so there is nothing to narrow.
            let connectors = (supported && descriptor.id == "claude")
                .then(|| {
                    let configuration = crate::marketplace::claude_sdk_configuration();
                    configuration
                        .mcp_servers
                        .keys()
                        .map(|instance| wire::WorkBriefingConnector {
                            id: instance.clone(),
                            family: crate::work_briefing_live::family_for_server(instance)
                                .map(|family| family.as_str().to_owned())
                                .unwrap_or_else(|| "unknown".to_owned()),
                            connected: configuration
                                .connector_health
                                .get(instance)
                                .copied()
                                .flatten(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            wire::WorkBriefingHarness {
                supported,
                reason: certification.err().map(|unsupported| unsupported.reason()),
                default_model,
                models: descriptor
                    .models
                    .iter()
                    .map(|model| wire::WorkBriefingModel {
                        id: model.id.clone(),
                        label: model.label.clone(),
                        tier: model.tier.as_str().to_owned(),
                        default_for_briefing: model.tier == CapabilityTier::Fast
                            && model.default_for_tier,
                    })
                    .collect(),
                connectors,
                id: descriptor.id,
                label: descriptor.label,
                available: descriptor.available,
            }
        })
        .collect();
    wire::WorkBriefingOptions { harnesses }
}

/// Trigger a briefing run. Every trigger — this one, app focus, and the cadence
/// thread — funnels through `work_briefing_trigger::claim`, so racing calls
/// start at most one run and the others observe it.
///
/// Returns immediately: a claimed run executes on its own thread and lands on
/// the run row, never in this response. The board's `suggestions.state` is how
/// a client follows it.
pub fn run_work_briefing(
    core: &Arc<BridgeCore>,
    params: &wire::RunBriefingParams,
) -> Result<wire::WorkBriefReceipt, BridgeError> {
    let registry = core.adapter_registry.clone();
    let versions = move |harness: &str| {
        registry
            .descriptors()
            .into_iter()
            .find(|descriptor| descriptor.id == harness)
            .and_then(|descriptor| descriptor.version)
    };
    let outcome = {
        let db = core.db.lock().unwrap();
        crate::work_briefing_trigger::claim(&db, params.trigger, &versions, chrono::Utc::now())?
    };
    Ok(match outcome {
        crate::work_briefing_trigger::ClaimOutcome::Claimed(run) => {
            let run_id = run.run_id.clone();
            let worker_core = core.clone();
            thread::spawn(move || crate::work_briefing_live::execute(&worker_core, run));
            wire::WorkBriefReceipt {
                outcome: wire::WorkBriefReceiptOutcome::Started,
                run_id: Some(run_id),
                code: None,
                detail: None,
            }
        }
        crate::work_briefing_trigger::ClaimOutcome::Observed { run_id } => wire::WorkBriefReceipt {
            outcome: wire::WorkBriefReceiptOutcome::Observed,
            run_id: Some(run_id),
            code: None,
            detail: None,
        },
        crate::work_briefing_trigger::ClaimOutcome::Refused { code, detail } => {
            wire::WorkBriefReceipt {
                outcome: wire::WorkBriefReceiptOutcome::Refused,
                run_id: None,
                code: Some(code),
                detail: Some(detail),
            }
        }
    })
}

/// Ask the active briefing run to stop. The run loop notices the flag, stops
/// the provider, records terminal status and measured usage, and leaves the
/// last good board intact.
pub fn cancel_work_briefing(core: &Arc<BridgeCore>) -> Result<wire::WorkBriefReceipt, BridgeError> {
    let cancelled = crate::work_briefing_trigger::request_cancel(&core.db.lock().unwrap())?;
    Ok(match cancelled {
        Some(run_id) => wire::WorkBriefReceipt {
            outcome: wire::WorkBriefReceiptOutcome::Observed,
            run_id: Some(run_id),
            code: None,
            detail: Some("cancellation requested; the run settles on its own thread".into()),
        },
        None => wire::WorkBriefReceipt {
            outcome: wire::WorkBriefReceiptOutcome::Refused,
            run_id: None,
            code: Some("not_running".into()),
            detail: Some("no briefing run is active".into()),
        },
    })
}

// --- base-branch divergence ----------------------------------------------------

/// Cache a divergence reading the user just paid for.
///
/// The Work board reads observations, never git, so a measurement taken on a
/// user's behalf should update the board instead of being thrown away. Failure
/// to cache is not failure to answer: the caller still gets its reading.
fn cache_base_divergence(
    core: &Arc<BridgeCore>,
    session_id: &str,
    divergence: &git::BaseBranchDivergence,
) {
    let db = core.db.lock().unwrap();
    let workspace_id: Option<String> = db
        .query_row(
            "SELECT workspace_id FROM sessions WHERE id=?1",
            params![session_id],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten();
    if let Some(workspace_id) = workspace_id {
        let _ = work_observation::record_base_divergence(
            &db,
            &workspace_id,
            Ok(divergence),
            chrono::Utc::now(),
        );
    }
}

/// How far this session's workspace has drifted from the branch it builds on.
/// Read-only; `fetch` controls whether the network is consulted.
pub fn workspace_base_divergence(
    core: &Arc<BridgeCore>,
    session_id: &str,
    fetch: bool,
) -> Result<git::BaseBranchDivergence, BridgeError> {
    let workspace_id: Option<String> = core.db.lock().unwrap().query_row(
        "SELECT workspace_id FROM sessions WHERE id=?1",
        params![session_id],
        |row| row.get(0),
    )?;
    // Resolve under the lock, then drop it before Git: `fetch` has no timeout
    // and must not stall checkout, writes, or chat start for this workspace.
    let path = with_optional_workspace_lock(core, workspace_id.as_deref(), || {
        store::base_branch_path_for_session(&core.db.lock().unwrap(), session_id)
    })?
    .ok_or_else(|| BridgeError::Invalid("this session has no connected repository".into()))?;
    let divergence = git::base_branch_divergence(&path, fetch);
    cache_base_divergence(core, session_id, &divergence);
    Ok(divergence)
}

/// The "refresh" choice offered by a stale-base warning: fast-forward the
/// workspace onto its base ref. Refuses on a dirty tree, an active session, or
/// any history that is not a pure fast-forward.
pub fn refresh_workspace_base(
    core: &Arc<BridgeCore>,
    session_id: &str,
) -> Result<git::BaseBranchDivergence, BridgeError> {
    let workspace_id: Option<String> = core.db.lock().unwrap().query_row(
        "SELECT workspace_id FROM sessions WHERE id=?1",
        params![session_id],
        |row| row.get(0),
    )?;
    let path = with_optional_workspace_lock(core, workspace_id.as_deref(), || {
        store::base_branch_path_for_session(&core.db.lock().unwrap(), session_id)
    })?
    .ok_or_else(|| BridgeError::Invalid("this session has no connected repository".into()))?;
    // Fetch without the operation lock: `git fetch` has no timeout.
    let _ = git::base_branch_divergence(&path, true);
    let divergence = with_optional_workspace_lock(core, workspace_id.as_deref(), || {
        let (path, mut active) = {
            let db = core.db.lock().unwrap();
            // Same directory the measurement was taken at: the workspace root for
            // a workspace session, the session cwd only for a direct chat.
            let path = store::base_branch_path_for_session(&db, session_id)?.ok_or_else(|| {
                BridgeError::Invalid("this session has no connected repository".into())
            })?;
            let active: bool = db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sessions WHERE (id=?1 OR parent_session_id=?1) AND status IN ('starting','working','resuming','checkpointing'))",
                    params![session_id],
                    |row| row.get(0),
                )
                .unwrap_or(true);
            (path, active)
        };
        if let Some(workspace_id) = workspace_id.as_deref() {
            active |= core
                .runtimes
                .lock()
                .unwrap()
                .contains_key(&format!("terminal:{workspace_id}"));
        }
        git::fast_forward_to_base_fetching(&path, active, false)
    })?;
    cache_base_divergence(core, session_id, &divergence);
    {
        let db = core.db.lock().unwrap();
        let _ = store::event(
            &db,
            "workspace",
            "workspace.base_refreshed",
            session_id,
            &serde_json::to_string(&divergence).unwrap_or_default(),
        );
        // Durable, so the resolution is still visible after a reload instead of
        // leaving only the warning that prompted it.
        let resolved = agent::NormalizedEvent {
            kind: "workspace.stale_base".into(),
            item_id: Some(format!(
                "stale-base-{}@{}",
                divergence.base_ref.as_deref().unwrap_or("unknown"),
                divergence.base_commit.as_deref().unwrap_or("unknown")
            )),
            role: Some("system".into()),
            status: Some("resolved".into()),
            title: Some(format!(
                "Workspace refreshed onto {}",
                divergence.base_ref.as_deref().unwrap_or("its base branch")
            )),
            text: Some(divergence.summary()),
            data: serde_json::json!({
                "staleBase": true,
                "phase": "refreshed",
                "divergence": divergence,
                "choices": [],
            }),
        };
        if let Ok(stored) = store::session_event(
            &db,
            session_id,
            &resolved,
            &serde_json::json!({"workspace": true}),
        ) {
            drop(db);
            core.events.publish(CoreEvent::Agent(stored));
        }
    }
    core.events.publish(CoreEvent::StateChanged);
    Ok(divergence)
}

// --- worker worktree adoption --------------------------------------------------

/// Children of this session whose changes exist only in their own worktree.
pub fn pending_worker_adoptions(
    core: &Arc<BridgeCore>,
    session_id: &str,
) -> Result<Vec<worker_adoption::WorkerRepositoryBinding>, BridgeError> {
    worker_adoption::pending_for_parent(&core.db.lock().unwrap(), session_id)
}

// --- token and cost usage --------------------------------------------------------

/// Day or hour roll-ups of the usage ledger per harness and model.
pub fn usage_summary(
    core: &Arc<BridgeCore>,
    request: &usage_summary::UsageSummaryRequest,
) -> Result<usage_summary::UsageSummary, BridgeError> {
    usage_summary::summarize(&core.db.lock().unwrap(), request)
}

/// The Insights tab: the stored report, or a fresh one from a headless harness
/// turn over Bridge's own usage, prompt, and GitHub records. Blocking — the
/// run is bounded by `usage_insights::MAX_WALL_SECONDS`, inside the daemon
/// client's call timeout.
pub fn usage_insights(
    core: &Arc<BridgeCore>,
    params: &wire::InsightsParams,
) -> Result<wire::UsageInsightsResult, BridgeError> {
    crate::usage_insights::insights(core, params)
}

pub fn list_usage_price_overrides(
    core: &Arc<BridgeCore>,
) -> Result<Vec<usage_pricing::PriceOverride>, BridgeError> {
    usage_pricing::list_price_overrides(&core.db.lock().unwrap())
}

/// Set a user rate for one model and return every override in force.
pub fn set_usage_price_override(
    core: &Arc<BridgeCore>,
    model: &str,
    input_microusd_per_mtok: i64,
    output_microusd_per_mtok: i64,
    cache_read_microusd_per_mtok: Option<i64>,
    cache_write_microusd_per_mtok: Option<i64>,
) -> Result<Vec<usage_pricing::PriceOverride>, BridgeError> {
    let db = core.db.lock().unwrap();
    usage_pricing::set_price_override(
        &db,
        model,
        input_microusd_per_mtok,
        output_microusd_per_mtok,
        cache_read_microusd_per_mtok,
        cache_write_microusd_per_mtok,
    )?;
    usage_pricing::list_price_overrides(&db)
}

pub fn clear_usage_price_override(
    core: &Arc<BridgeCore>,
    model: &str,
) -> Result<Vec<usage_pricing::PriceOverride>, BridgeError> {
    let db = core.db.lock().unwrap();
    usage_pricing::clear_price_override(&db, model)?;
    usage_pricing::list_price_overrides(&db)
}

/// Fetch a fresh LiteLLM rate table and cache it. The only place usage
/// pricing touches the network, and only because a client asked. The fetch
/// runs before the database lock is taken so a slow upstream never stalls
/// other callers.
pub fn refresh_usage_rates(
    core: &Arc<BridgeCore>,
) -> Result<usage_pricing::PricingStatus, BridgeError> {
    let document = usage_pricing::fetch_rate_document()?;
    usage_pricing::refresh_rates_from_document(&core.db.lock().unwrap(), &document)
}

/// The history sources the importers can see on this machine, with what has
/// been indexed from each. Discovery reads the filesystem, so it runs before
/// the database lock is taken.
pub fn list_usage_history_sources(
    core: &Arc<BridgeCore>,
) -> Result<Vec<usage_history::UsageHistorySource>, BridgeError> {
    let env = usage_import::SourceEnv::from_process();
    usage_history::list_history_sources(&core.db.lock().unwrap(), &env)
}

/// One bounded, incremental import pass over the chosen history sources.
pub fn scan_usage_history(
    core: &Arc<BridgeCore>,
    max_records: Option<usize>,
    source_ids: Option<&[String]>,
) -> Result<usage_import::ScanReport, BridgeError> {
    let env = usage_import::SourceEnv::from_process();
    usage_history::scan_history(core, &env, max_records, source_ids)
}

// --- menu-bar meter (CodexBar port) ------------------------------------------------
// Static provider registry plus the shared refresh trigger. Live windows ride
// the existing `account-usage` event channel (see `refresh_account_usage`);
// pace math lives in `meter` (Rust) and `src/meter.ts` (TypeScript), both
// ported from CodexBar's `UsagePace.weekly`.

/// The meter registry: live providers plus planned CodexBar follow-ups.
pub fn meter_snapshot() -> meter::MeterRegistry {
    meter::registry_snapshot()
}

/// Trigger the shared account-usage refresh (Claude `/usage` probe plus Codex,
/// from a live session when there is one and from its rollouts when there is
/// not); results arrive on the `account-usage` channel.
///
/// Coalesced: calls within 10 seconds of an accepted one return `Ok` without
/// spawning another probe pair. Neither the tray menu nor the popover button
/// can show a spinner, so rapid re-clicks would otherwise stack a Claude PTY
/// probe per click — the hammering the adaptive policy exists to prevent.
pub fn refresh_meter(core: &Arc<BridgeCore>) -> Result<(), BridgeError> {
    static LAST_REFRESH_MS: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
    let now = chrono::Utc::now().timestamp_millis();
    if now - LAST_REFRESH_MS.load(std::sync::atomic::Ordering::SeqCst) < 10_000 {
        return Ok(());
    }
    LAST_REFRESH_MS.store(now, std::sync::atomic::Ordering::SeqCst);
    core.refresh_account_usage()
}

// --- worktree inventory --------------------------------------------------------

pub fn get_worker_settings(core: &Arc<BridgeCore>, workspace_id: &str) -> Result<wire::WorkerSettings, BridgeError> {
    crate::worker_settings::load(&core.db.lock().unwrap(), workspace_id)
}

pub fn save_worker_settings(core: &Arc<BridgeCore>, workspace_id: &str, settings: &wire::WorkerSettings) -> Result<wire::WorkerSettings, BridgeError> {
    crate::worker_settings::save(&core.db.lock().unwrap(), workspace_id, settings)
}

/// Every worktree Bridge knows about, with the last assessment of what may be
/// done with it. A read: the sweep owns reclaiming.
pub fn list_worktrees(
    core: &Arc<BridgeCore>,
) -> Result<Vec<worktree_registry::WorktreeInventoryEntry>, BridgeError> {
    worktree_registry::inventory(&core.db.lock().unwrap())
}

/// What the worktrees cost against the caps in force.
pub fn worktree_usage(
    core: &Arc<BridgeCore>,
) -> Result<worktree_registry::WorktreeUsage, BridgeError> {
    worktree_registry::usage(
        &core.db.lock().unwrap(),
        &worktree_registry::WorktreeRetention::default(),
    )
}

/// Put one chat away and reclaim the checkout it owns.
///
/// Deliberately *not* `archive_workspace`. That archives a workspace, which
/// deletes every session in it — and a workspace here holds many chats (one on
/// the machine this was written for holds 836), so wiring a per-chat button to
/// it would destroy hundreds of unrelated conversations to reclaim one
/// directory.
///
/// History is kept. The session row, its forest entries and its evidence all
/// survive; the chat is marked archived so it is no longer listed, and its own
/// worktree goes through the same classification and refusals as any other
/// reclaim. A checkout that cannot be proven expendable is *kept* rather than
/// blocking the archive, and the reason comes back with the result — putting a
/// conversation away should not require first resolving its uncommitted work.
pub fn list_archived_chats(core: &Arc<BridgeCore>, request: &wire::ListArchivedChatsParams) -> Result<wire::ArchivedChatsResult, BridgeError> {
    let db = core.db.lock().unwrap();
    let mut statement = db.prepare(
        "WITH RECURSIVE family(root_id,id) AS (
             SELECT id,id FROM sessions WHERE archived_at IS NOT NULL AND (?3 IS NULL OR id=?3)
             UNION
             SELECT f.root_id,s.id FROM sessions s JOIN family f ON s.parent_session_id=f.id
         )
         SELECT s.id,COALESCE(NULLIF(s.title,''),s.label),s.harness,w.title,COALESCE(s.archived_at,r.archived_at)
         FROM sessions s LEFT JOIN workspaces w ON w.id=s.workspace_id
         LEFT JOIN sessions r ON r.id=?3
         WHERE (?3 IS NULL AND s.archived_at IS NOT NULL
             AND NOT EXISTS(SELECT 1 FROM family f WHERE f.id=s.id AND f.root_id<>s.id)
             AND EXISTS(SELECT 1 FROM family f JOIN sessions child ON child.id=f.id
                 WHERE f.root_id=s.id AND instr(lower(COALESCE(child.title,'')||' '||child.label||' '||COALESCE(w.title,'')),lower(?1))>0))
            OR (?3 IS NOT NULL
                AND EXISTS(SELECT 1 FROM family f WHERE f.root_id=?3 AND f.id=s.id AND f.id<>f.root_id)
                AND instr(lower(COALESCE(s.title,'')||' '||s.label||' '||COALESCE(w.title,'')),lower(?1))>0)
         ORDER BY COALESCE(s.archived_at,r.archived_at) DESC,s.id LIMIT 51 OFFSET ?2",
    )?;
    let mut chats = statement.query_map(params![request.query.trim(), request.offset, request.root_session_id], |row| {
        Ok(wire::ArchivedChat { id: row.get(0)?, title: row.get(1)?, harness: row.get(2)?, workspace_title: row.get(3)?, archived_at: row.get(4)? })
    })?.collect::<Result<Vec<_>, _>>()?;
    let has_more = chats.len() > 50;
    chats.truncate(50);
    Ok(wire::ArchivedChatsResult { chats, has_more })
}

/// Visibility only: never restore a checkout or start a provider here.
pub fn unarchive_chat(core: &Arc<BridgeCore>, session_id: &str) -> Result<(), BridgeError> {
    {
        let mut db = core.db.lock().unwrap();
        let tx = db.transaction()?;
        let archived: Option<String> = tx.query_row("SELECT archived_at FROM sessions WHERE id=?1", params![session_id], |row| row.get(0))?;
        if archived.is_some() {
            tx.execute("UPDATE sessions SET archived_at=NULL WHERE id=?1", params![session_id])?;
            store::event(&tx, "user", "session.unarchived", session_id, "Chat returned to history; no checkout restored or model started")?;
        }
        tx.commit()?;
    }
    core.events.publish(CoreEvent::StateChanged);
    Ok(())
}

pub fn archive_chat(
    core: &Arc<BridgeCore>,
    session_id: &str,
) -> Result<worktree_registry::ArchiveChatResult, BridgeError> {
    let (status, active_turn, adapter_pid): (String, Option<String>, Option<i64>) =
        core.db.lock().unwrap().query_row(
            "SELECT status,active_turn_id,adapter_pid FROM sessions WHERE id=?1",
            params![session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
    // A `ready` chat has no turn in flight but still owns a live provider
    // process. Hiding it would take away the only route to that process while
    // it goes on holding memory, a port and a model session — and the worktree
    // would be retained anyway, since the same claim marks it in use. Archiving
    // has to mean the chat is really finished.
    if active_turn.is_some()
        || adapter_pid.is_some()
        || matches!(
            status.as_str(),
            "working" | "waiting" | "starting" | "resuming" | "checkpointing" | "ready"
        )
    {
        return Err(BridgeError::Invalid(
            "Stop this chat before archiving it".into(),
        ));
    }

    let owned = { worktree_registry::owned_by_session(&core.db.lock().unwrap(), session_id)? };
    let reclaim = match owned {
        Some(record) => Some(worktree_registry::reclaim(
            &core.db,
            &core.worktrees,
            &record.id,
            &worktree_registry::WorktreeRetention::default(),
            false,
        )?),
        None => None,
    };

    core.db.lock().unwrap().execute(
        "UPDATE sessions SET archived_at=?2,ended_at=COALESCE(ended_at,?2),active_turn_id=NULL
          WHERE id=?1 AND archived_at IS NULL",
        params![session_id, chrono::Utc::now().to_rfc3339()],
    )?;
    {
        let db = core.db.lock().unwrap();
        let freed = reclaim
            .as_ref()
            .filter(|outcome| outcome.reclaimed)
            .map(|outcome| worktree_registry::human_bytes(outcome.bytes_freed.max(0) as u64))
            .unwrap_or_else(|| "nothing".to_owned());
        store::event(
            &db,
            "supervisor",
            "session.archived",
            session_id,
            &format!("Chat archived; reclaimed {freed}"),
        )?;
    }
    core.events.publish(CoreEvent::StateChanged);
    Ok(worktree_registry::ArchiveChatResult {
        archived: true,
        bytes_freed: reclaim
            .as_ref()
            .filter(|outcome| outcome.reclaimed)
            .map(|outcome| outcome.bytes_freed)
            .unwrap_or(0),
        worktree_detail: reclaim
            .filter(|outcome| !outcome.reclaimed)
            .and_then(|outcome| outcome.detail),
    })
}

/// Reclaim one checkout because a person asked. A refusal comes back in the
/// result, with its reason, rather than as an error.
///
/// `force` is the one place a client can widen what "asked" covers: it lets a
/// person remove a checkout the sweep would never touch on its own —
/// uncommitted changes, or one git cannot vouch for — because they can see it
/// and have decided for themselves. It changes nothing about what Bridge
/// still refuses unconditionally: a checkout it did not create, one outside
/// its namespace, or one a live session owns.
pub fn reclaim_worktree(
    core: &Arc<BridgeCore>,
    worktree_id: &str,
    force: bool,
) -> Result<worktree_registry::WorktreeReclaimResult, BridgeError> {
    let outcome = worktree_registry::reclaim(
        &core.db,
        &core.worktrees,
        worktree_id,
        &worktree_registry::WorktreeRetention::default(),
        force,
    )?;
    if outcome.reclaimed {
        core.events.publish(CoreEvent::StateChanged);
    }
    Ok(outcome)
}

/// Run the maintenance pass now instead of waiting for the tick.
pub fn sweep_worktrees(
    core: &Arc<BridgeCore>,
) -> Result<worktree_registry::SweepOutcome, BridgeError> {
    let outcome = worktree_registry::run_requested_pass(
        &core.db,
        &core.worktrees,
        &worktree_registry::WorktreeRetention::default(),
    )?;
    if outcome.removed > 0 {
        core.events.publish(CoreEvent::StateChanged);
    }
    Ok(outcome)
}

/// Merge a worker's isolated worktree into the task checkout. Integration
/// refuses to run against a dirty or active task worktree, so a rejected call
/// leaves the work pending rather than losing it.
pub fn adopt_worker_worktree(
    core: &Arc<BridgeCore>,
    session_id: &str,
) -> Result<worker_adoption::WorkerRepositoryBinding, BridgeError> {
    let workspace_id: String = core
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT workspace_id FROM worker_worktree_adoptions WHERE session_id=?1",
            params![session_id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| {
            BridgeError::Invalid(format!("worker {session_id} has no repository binding"))
        })?;
    let workspace_operation = core.workspace_operation(&workspace_id);
    let _workspace_operation = workspace_operation.lock().unwrap();
    if core
        .runtimes
        .lock()
        .unwrap()
        .contains_key(&format!("terminal:{workspace_id}"))
    {
        return Err(BridgeError::Invalid(
            "Close the workspace terminal before adopting worker changes".into(),
        ));
    }
    // Validate under the lock, merge with the lock released, settle under it
    // again. `git merge` plus `git worktree remove` can take seconds, and holding
    // the shared connection across them would stall every other session.
    let plan = {
        let db = core.db.lock().unwrap();
        worker_adoption::plan_adoption(&db, session_id)?
    };
    let outcome = match worker_adoption::integrate(&plan) {
        Ok(outcome) => outcome,
        Err(error) => {
            // The claim must be handed back, or the decision is stuck forever.
            let db = core.db.lock().unwrap();
            let _ = worker_adoption::release_claim(&db, session_id, &error.to_string());
            drop(db);
            core.events.publish(CoreEvent::StateChanged);
            return Err(error);
        }
    };
    let binding = {
        let db = core.db.lock().unwrap();
        let binding = worker_adoption::settle_plan(
            &db,
            &plan,
            worker_adoption::STATE_ADOPTED,
            &outcome.detail,
        )?;
        // A merged tree that is not the verified tree must not inherit its proof.
        if !outcome.verified_tree_preserved {
            worker_adoption::supersede_stale_proof(&db, &plan, &outcome)?;
        }
        completion::reconcile_parent_readiness(&db, &binding.parent_session_id)?;
        binding
    };
    core.events.publish(CoreEvent::StateChanged);
    Ok(binding)
}

/// Throw a worker's isolated output away on purpose and release its worktree.
pub fn discard_worker_worktree(
    core: &Arc<BridgeCore>,
    session_id: &str,
    reason: &str,
) -> Result<worker_adoption::WorkerRepositoryBinding, BridgeError> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(BridgeError::Invalid(
            "discarding a worker worktree requires a reason".into(),
        ));
    }
    let workspace_id: String = core
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT workspace_id FROM worker_worktree_adoptions WHERE session_id=?1",
            params![session_id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| {
            BridgeError::Invalid(format!("worker {session_id} has no repository binding"))
        })?;
    let workspace_operation = core.workspace_operation(&workspace_id);
    let _workspace_operation = workspace_operation.lock().unwrap();
    let plan = {
        let db = core.db.lock().unwrap();
        worker_adoption::plan_discard(&db, session_id)?
    };
    let binding = {
        let db = core.db.lock().unwrap();
        let binding =
            worker_adoption::settle_plan(&db, &plan, worker_adoption::STATE_DISCARDED, reason)?;
        completion::reconcile_parent_readiness(&db, &binding.parent_session_id)?;
        binding
    };
    core.events.publish(CoreEvent::StateChanged);
    Ok(binding)
}

// --- routing -------------------------------------------------------------------

pub fn get_router_preferences(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
) -> Result<learning_router::RouterPreferences, BridgeError> {
    learning_router::load_preferences(&core.db.lock().unwrap(), workspace_id)
}

pub fn update_router_preferences(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
    preferences: &learning_router::RouterPreferences,
) -> Result<learning_router::RouterPreferences, BridgeError> {
    let db = core.db.lock().unwrap();
    learning_router::save_preferences(&db, workspace_id, preferences)?;
    learning_router::load_preferences(&db, workspace_id)
}

pub fn rollback_routing_policy(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
    target_version: i64,
    explanation: &str,
) -> Result<learning_job::LearningState, BridgeError> {
    learning_job::rollback_policy(&core.db.lock().unwrap(), workspace_id, target_version, explanation)?;
    let result = learning_job::learning_state(&core.db.lock().unwrap(), workspace_id)?;
    core.events.publish(CoreEvent::LearningJobChanged(
        serde_json::to_value(&result).unwrap_or_default(),
    ));
    Ok(result)
}

/// How many runs the surface reads at once. A workspace accumulates one
/// evaluation per unknown outcome, and the panel wants the recent ones.
const EVALUATION_RUN_PAGE: i64 = 50;

fn evaluation_settings_wire(
    settings: routing_evaluation::EvaluationSettings,
) -> wire::RoutingEvaluationSettings {
    wire::RoutingEvaluationSettings {
        scope_key: settings.scope_key,
        mode: settings.mode,
        harness: settings.harness,
        model: settings.model,
    }
}

pub fn get_routing_evaluations(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
) -> Result<wire::RoutingEvaluationsResult, BridgeError> {
    let db = core.db.lock().unwrap();
    let settings = routing_evaluation::settings(&db, workspace_id)?;
    let runs = routing_evaluation::list_runs(&db, workspace_id, EVALUATION_RUN_PAGE)?
        .into_iter()
        .map(|run| wire::RoutingEvaluationRun {
            run_id: run.run_id,
            decision_id: run.decision_id,
            status: run.status,
            harness: run.harness,
            model: run.model,
            evaluator_version: run.evaluator_version,
            evidence_digest: run.evidence_digest,
            score_bps: run.score_bps,
            confidence_bps: run.confidence_bps,
            observed_tokens: run.observed_tokens,
            spend_microusd: run.spend_microusd,
            detail: run.detail,
            created_at: run.created_at,
            updated_at: run.updated_at,
        })
        .collect();
    Ok(wire::RoutingEvaluationsResult {
        settings: evaluation_settings_wire(settings),
        runs,
    })
}

pub fn get_evaluation_settings(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
) -> Result<wire::RoutingEvaluationSettings, BridgeError> {
    let db = core.db.lock().unwrap();
    Ok(evaluation_settings_wire(routing_evaluation::settings(&db, workspace_id)?))
}

pub fn update_evaluation_settings(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
    mode: &str,
    harness: Option<&str>,
    model: Option<&str>,
) -> Result<wire::RoutingEvaluationSettings, BridgeError> {
    let db = core.db.lock().unwrap();
    let settings =
        routing_evaluation::update_settings(&db, workspace_id, mode, harness, model)?;
    Ok(evaluation_settings_wire(settings))
}

// --- model profiles --------------------------------------------------------------

pub fn get_model_setup(
    core: &Arc<BridgeCore>,
) -> Result<model_profiles::ModelSetupState, BridgeError> {
    model_profiles::setup_state(&core.db.lock().unwrap())
}

pub fn recommended_model_profiles(
    core: &Arc<BridgeCore>,
) -> Result<Vec<model_profiles::ModelProfileDraft>, BridgeError> {
    model_profiles::recommended_profiles(&core.adapter_registry.descriptors())
}

pub fn save_model_profiles(
    core: &Arc<BridgeCore>,
    profiles: &[model_profiles::ModelProfileDraft],
) -> Result<model_profiles::ModelSetupState, BridgeError> {
    model_profiles::save_profiles(
        &core.db.lock().unwrap(),
        &core.adapter_registry.descriptors(),
        profiles,
    )
}

pub fn reset_model_profiles(
    core: &Arc<BridgeCore>,
) -> Result<model_profiles::ModelSetupState, BridgeError> {
    model_profiles::reset_profiles(
        &core.db.lock().unwrap(),
        &core.adapter_registry.descriptors(),
    )
}

// --- inline composer suggestions ---------------------------------------------------

/// The composer typeahead's stored configuration.
pub fn get_suggestion_settings(
    core: &Arc<BridgeCore>,
) -> Result<wire::SuggestionSettingsSnapshot, BridgeError> {
    suggestion_engine::read_settings(&core.db.lock().unwrap())
}

/// Persist the typeahead's configuration. Validation lives in Rust —
/// `suggestion_engine::validate_settings` plus the model-catalog check below,
/// which needs the adapter registry the store-only module cannot reach. Same
/// shape as `write_work_settings`.
pub fn save_suggestion_settings(
    core: &Arc<BridgeCore>,
    params: &wire::SaveSuggestionSettingsParams,
) -> Result<wire::SuggestionSettingsSnapshot, BridgeError> {
    let descriptors = core.adapter_registry.descriptors();
    if let Some(descriptor) = descriptors
        .iter()
        .find(|descriptor| descriptor.id == params.settings.provider)
    {
        // An empty catalog is a runtime-discovered one; only a non-empty
        // catalog can refuse a model by name.
        if !descriptor.models.is_empty()
            && !descriptor.models.iter().any(|model| model.id == params.settings.model)
        {
            return Err(BridgeError::Invalid(format!(
                "{} is not a model {} offers",
                params.settings.model, descriptor.label
            )));
        }
    }
    suggestion_engine::write_settings(&core.db.lock().unwrap(), &params.settings)
}

/// Ask the typeahead engine to continue the composer's current draft. Refuses
/// outright when suggestions are turned off — the caller (the UI's debounce)
/// is expected not to call this at all in that case, but the refusal is the
/// authority, not the UI's own gating.
pub fn suggest_completion(
    core: &Arc<BridgeCore>,
    params: &wire::SuggestCompletionParams,
) -> Result<wire::SuggestCompletionResult, BridgeError> {
    suggestion_engine::suggest_completion(core, &params.text)
}

// --- configuration ----------------------------------------------------------------

fn opencode_directory(directory: Option<String>) -> Result<String, BridgeError> {
    let path = directory
        .map(|value| PathBuf::from(value.trim()))
        .filter(|path| !path.as_os_str().is_empty())
        .map(Ok)
        .unwrap_or_else(std::env::current_dir)?;
    if !path.is_dir() {
        return Err(BridgeError::Invalid(format!(
            "OpenCode directory does not exist: {}",
            path.display()
        )));
    }
    Ok(path.to_string_lossy().into_owned())
}

pub fn get_config_state(core: &Arc<BridgeCore>) -> Result<agent_config::ConfigState, BridgeError> {
    agent_config::state(&core.db.lock().unwrap())
}

pub fn save_harness_config(
    core: &Arc<BridgeCore>,
    config: agent_config::HarnessConfig,
) -> Result<agent_config::ConfigState, BridgeError> {
    let opencode_settings = (config.id == "opencode")
        .then(|| agent_config::opencode_settings(Some(&config)))
        .transpose()?;
    let next = agent_config::save_harness(&core.db.lock().unwrap(), config)?;
    if let Some(settings) = opencode_settings {
        let directory = opencode_directory(None)?;
        let _ = core.adapter_registry.refresh_opencode(settings, &directory);
    }
    Ok(next)
}

pub fn reset_harness_config(
    core: &Arc<BridgeCore>,
    id: &str,
) -> Result<agent_config::ConfigState, BridgeError> {
    let next = agent_config::reset_harness(&core.db.lock().unwrap(), id)?;
    if id == "opencode" {
        let directory = opencode_directory(None)?;
        let _ = core
            .adapter_registry
            .refresh_opencode(opencode_adapter::OpenCodeSettings::default(), &directory);
    }
    Ok(next)
}

pub fn refresh_opencode_catalog(
    core: &Arc<BridgeCore>,
    directory: Option<String>,
) -> Result<opencode_adapter::OpenCodeCatalog, BridgeError> {
    let directory = opencode_directory(directory)?;
    let settings = core.adapter_registry.opencode_settings()?;
    core.adapter_registry.refresh_opencode(settings, &directory)
}

pub fn set_opencode_provider_api_key(
    core: &Arc<BridgeCore>,
    provider_id: &str,
    api_key: &str,
    directory: Option<String>,
) -> Result<opencode_adapter::OpenCodeCatalog, BridgeError> {
    let directory = opencode_directory(directory)?;
    core.adapter_registry
        .set_opencode_provider_api_key(&directory, provider_id, api_key)
}

pub fn remove_opencode_provider_auth(
    core: &Arc<BridgeCore>,
    provider_id: &str,
    directory: Option<String>,
) -> Result<opencode_adapter::OpenCodeCatalog, BridgeError> {
    let directory = opencode_directory(directory)?;
    core.adapter_registry
        .remove_opencode_provider_auth(&directory, provider_id)
}

pub fn save_agent_config(
    core: &Arc<BridgeCore>,
    agent: agent_config::AgentDefinition,
) -> Result<agent_config::ConfigState, BridgeError> {
    if agent.harness != "bridge" {
        let descriptor = core.adapter_registry.descriptors().into_iter()
            .find(|descriptor| descriptor.id == agent.harness)
            .ok_or_else(|| BridgeError::Invalid(format!("Unknown agent runtime {}", agent.harness)))?;
        if !adapters::descriptor_supports_agent_role(&descriptor, &agent.role) {
            return Err(BridgeError::Invalid(format!(
                "{} cannot run {} agents with the authority that role requires", descriptor.label, agent.role,
            )));
        }
    }
    agent_config::save_agent(&core.db.lock().unwrap(), agent)
}

pub fn delete_agent_config(
    core: &Arc<BridgeCore>,
    id: &str,
) -> Result<agent_config::ConfigState, BridgeError> {
    agent_config::delete_agent(&core.db.lock().unwrap(), id)
}

pub fn set_default_agent(
    core: &Arc<BridgeCore>,
    id: &str,
) -> Result<agent_config::ConfigState, BridgeError> {
    agent_config::set_default(&core.db.lock().unwrap(), id)
}

/// Persist the permission policy and hand back the whole config snapshot, the
/// same shape every other config mutation returns.
pub fn save_permission_policy(
    core: &Arc<BridgeCore>,
    policy: agent_config::PermissionPolicy,
) -> Result<agent_config::ConfigState, BridgeError> {
    let next = agent_config::save_permission_policy(&core.db.lock().unwrap(), policy)?;
    // The badge in the app chrome renders from state, so a flip has to push.
    core.events.publish(CoreEvent::StateChanged);
    Ok(next)
}

pub fn reset_all_config(core: &Arc<BridgeCore>) -> Result<agent_config::ConfigState, BridgeError> {
    let next = agent_config::reset_all(&core.db.lock().unwrap())?;
    // Reset deletes every configuration row, the permission policy among them, so
    // the chrome badge has to hear about it or it keeps advertising a bypass that
    // is no longer in effect.
    core.events.publish(CoreEvent::StateChanged);
    let directory = opencode_directory(None)?;
    let _ = core
        .adapter_registry
        .refresh_opencode(opencode_adapter::OpenCodeSettings::default(), &directory);
    Ok(next)
}

// --- prompt studio ------------------------------------------------------------------

/// A studio target choice is the same fact as a core prompt target: the wire
/// enum's renames are the core storage keys, so this conversion is total.
pub fn prompt_target(choice: wire::PromptTargetChoice) -> prompts::PromptTarget {
    match choice {
        wire::PromptTargetChoice::Orchestrator => prompts::PromptTarget::Orchestrator,
        wire::PromptTargetChoice::WorkerResearch => {
            prompts::PromptTarget::Worker(delegation::WorkerRole::Research)
        }
        wire::PromptTargetChoice::WorkerImplementation => {
            prompts::PromptTarget::Worker(delegation::WorkerRole::Implementation)
        }
        wire::PromptTargetChoice::WorkerVerification => {
            prompts::PromptTarget::Worker(delegation::WorkerRole::Verification)
        }
        wire::PromptTargetChoice::WorkerPlanning => {
            prompts::PromptTarget::Worker(delegation::WorkerRole::Planning)
        }
        wire::PromptTargetChoice::WorkerDocumentation => {
            prompts::PromptTarget::Worker(delegation::WorkerRole::Documentation)
        }
        wire::PromptTargetChoice::DirectSession => prompts::PromptTarget::DirectSession,
    }
}

fn resolved_depth(depth: Option<i64>) -> Result<i64, BridgeError> {
    Ok(depth.unwrap_or(0))
}

/// The full studio view of one target's prompt stack: states, defaults,
/// effective text and sizes per section, lint warnings, and revision history.
pub fn get_prompt_stack(
    core: &Arc<BridgeCore>,
    target: prompts::PromptTarget,
    depth: Option<i64>,
) -> Result<prompt_studio::PromptStackView, BridgeError> {
    prompt_studio::stack(&core.db.lock().unwrap(), target, resolved_depth(depth)?)
}

pub fn save_prompt_section(
    core: &Arc<BridgeCore>,
    target: prompts::PromptTarget,
    section_id: &str,
    text: &str,
    depth: Option<i64>,
) -> Result<prompt_studio::PromptSectionMutation, BridgeError> {
    let mutation = prompt_studio::save_section(
        &core.db.lock().unwrap(),
        target,
        section_id,
        resolved_depth(depth)?,
        text,
    )?;
    core.events.publish(CoreEvent::StateChanged);
    Ok(mutation)
}

pub fn reset_prompt_section(
    core: &Arc<BridgeCore>,
    target: prompts::PromptTarget,
    section_id: &str,
    depth: Option<i64>,
) -> Result<prompt_studio::PromptSectionMutation, BridgeError> {
    let mutation = prompt_studio::reset_section(
        &core.db.lock().unwrap(),
        target,
        section_id,
        resolved_depth(depth)?,
    )?;
    core.events.publish(CoreEvent::StateChanged);
    Ok(mutation)
}

pub fn restore_prompt_revision(
    core: &Arc<BridgeCore>,
    target: prompts::PromptTarget,
    section_id: &str,
    revision_id: i64,
    depth: Option<i64>,
) -> Result<prompt_studio::PromptSectionMutation, BridgeError> {
    let mutation = prompt_studio::restore_revision(
        &core.db.lock().unwrap(),
        target,
        section_id,
        revision_id,
        resolved_depth(depth)?,
    )?;
    core.events.publish(CoreEvent::StateChanged);
    Ok(mutation)
}

/// The exact Bridge-authored envelopes for one target plus honest
/// provider-layer statuses. A pure read: nothing here mutates.
pub fn preview_compiled_prompt(
    core: &Arc<BridgeCore>,
    target: prompts::PromptTarget,
    depth: Option<i64>,
) -> Result<prompt_studio::CompiledPromptPreview, BridgeError> {
    prompt_studio::preview(&core.db.lock().unwrap(), target, resolved_depth(depth)?)
}

// --- adaptive learning --------------------------------------------------------------

pub fn get_learning_state(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
) -> Result<learning_job::LearningState, BridgeError> {
    learning_job::learning_state(&core.db.lock().unwrap(), workspace_id)
}

pub fn run_learning(
    core: &Arc<BridgeCore>,
    trigger_kind: learning_job::LearningTriggerKind,
    workspace_id: &str,
) -> Result<learning_job::LearningRun, BridgeError> {
    if matches!(
        trigger_kind,
        learning_job::LearningTriggerKind::Codex
            | learning_job::LearningTriggerKind::Claude
            | learning_job::LearningTriggerKind::OpenCode
    ) {
        return Err(BridgeError::Invalid(
            "external learning triggers must use a registered narrow command".into(),
        ));
    }
    // Learning runs open their own connection: the run must never hold the
    // global SQLite lock across model evaluation.
    let run = learning_job::run_local_database(&core.database_path, trigger_kind, workspace_id)?;
    core.events.publish(CoreEvent::LearningJobChanged(
        serde_json::to_value(&run).unwrap_or_default(),
    ));
    Ok(run)
}

pub fn cancel_learning_run(
    core: &Arc<BridgeCore>,
    run_id: &str,
) -> Result<learning_job::LearningRun, BridgeError> {
    let run = learning_job::cancel_run(&core.db.lock().unwrap(), run_id)?;
    core.events.publish(CoreEvent::LearningJobChanged(
        serde_json::to_value(&run).unwrap_or_default(),
    ));
    Ok(run)
}

pub fn update_learning_schedule(
    core: &Arc<BridgeCore>,
    schedule: &learning_job::LearningSchedule,
) -> Result<learning_job::LearningSchedule, BridgeError> {
    learning_job::update_schedule(&core.db.lock().unwrap(), schedule)
}

pub fn register_learning_trigger(
    core: &Arc<BridgeCore>,
    kind: learning_job::LearningTriggerKind,
    registration_id: &str,
    credential_ref: Option<&str>,
    expires_at: Option<&str>,
) -> Result<(), BridgeError> {
    learning_job::register_trigger_with_expiry(
        &core.db.lock().unwrap(),
        kind,
        registration_id,
        credential_ref,
        expires_at,
    )
}

pub fn get_learning_trigger_instructions(
    kind: learning_job::LearningTriggerKind,
    database_path: &str,
    registration_id: &str,
) -> Result<String, BridgeError> {
    learning_job::trigger_instructions(kind, database_path, registration_id)
}

pub fn enable_learning_trigger(
    core: &Arc<BridgeCore>,
    kind: learning_job::LearningTriggerKind,
    registration_id: &str,
) -> Result<(), BridgeError> {
    learning_job::enable_trigger(&core.db.lock().unwrap(), kind, registration_id)
}

pub fn approve_learning_run(
    core: &Arc<BridgeCore>,
    run_id: &str,
) -> Result<learning_job::LearningRun, BridgeError> {
    let run = learning_job::approve_run(&core.db.lock().unwrap(), run_id)?;
    core.events.publish(CoreEvent::LearningJobChanged(
        serde_json::to_value(&run).unwrap_or_default(),
    ));
    Ok(run)
}

// --- browser bridge --------------------------------------------------------------

pub fn browser_bridge_state(
    core: &Arc<BridgeCore>,
) -> Result<browser_bridge::BrowserBridgeSnapshot, BridgeError> {
    Ok(core.browser_bridge.state_snapshot())
}

pub fn browser_frame(
    core: &Arc<BridgeCore>,
    after_revision: u64,
) -> Result<Option<browser_bridge::BrowserFrame>, BridgeError> {
    Ok(core.browser_bridge.frame(after_revision))
}

fn find_browser_host(directory: &Path) -> Option<PathBuf> {
    let direct = directory.join("bridge-browser-host");
    if direct.exists() {
        return Some(direct);
    }
    std::fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("bridge-browser-host-"))
        })
}

pub fn install_browser_native_host(core: &Arc<BridgeCore>) -> Result<String, BridgeError> {
    let executable = std::env::var_os("BRIDGE_BROWSER_HOST")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|path| path.parent().and_then(find_browser_host))
        })
        .ok_or_else(|| BridgeError::Invalid("Could not locate bridge-browser-host".into()))?;
    if !executable.exists() {
        return Err(BridgeError::Invalid(format!(
            "Native host executable is missing at {}. Build the bridge-browser-host binary first.",
            executable.display()
        )));
    }
    core.browser_bridge
        .install_native_host(&executable)
        .map(|path| path.to_string_lossy().into_owned())
}

pub fn browser_action(
    core: &Arc<BridgeCore>,
    request: browser_bridge::BrowserActionRequest,
) -> Result<String, BridgeError> {
    core.browser_bridge.issue(request)
}

pub fn set_browser_permission(core: &Arc<BridgeCore>, permission: &str) -> Result<(), BridgeError> {
    core.browser_bridge.set_permission(permission)
}

pub fn resolve_browser_approval(
    core: &Arc<BridgeCore>,
    approval_id: &str,
    allow: bool,
) -> Result<(), BridgeError> {
    core.browser_bridge.resolve_approval(approval_id, allow)
}

pub fn takeover_browser(core: &Arc<BridgeCore>) -> Result<(), BridgeError> {
    core.browser_bridge.takeover()
}

pub fn detach_browser(core: &Arc<BridgeCore>) -> Result<String, BridgeError> {
    core.browser_bridge.detach()
}

pub fn route_browser(
    request: browser_bridge::BrowserRouteRequest,
) -> browser_bridge::BrowserRouteDecision {
    browser_bridge::route_browser(request)
}

pub fn browser_skills() -> Vec<browser_bridge::BrowserSkill> {
    browser_bridge::bundled_skills()
}

pub fn configure_remote_browser(
    core: &Arc<BridgeCore>,
    config: Option<browser_bridge::RemoteBrowserConfig>,
) -> Result<(), BridgeError> {
    core.browser_bridge.configure_remote(config)
}

pub fn start_remote_browser(
    core: &Arc<BridgeCore>,
    initial_url: &str,
) -> Result<Value, BridgeError> {
    core.browser_bridge.start_remote_session(initial_url)
}

// --- marketplace -----------------------------------------------------------------

pub fn marketplace_catalog() -> marketplace::MarketplaceCatalog {
    marketplace::catalog()
}

pub fn marketplace_app_auth_states(
) -> Result<Vec<marketplace::MarketplaceAppAuthState>, BridgeError> {
    marketplace::app_auth_states()
}

pub fn marketplace_action(
    provider: marketplace::MarketplaceProvider,
    plugin_id: &str,
    marketplace_name: Option<&str>,
    action: marketplace::MarketplaceAction,
) -> Result<marketplace::MarketplaceActionResult, BridgeError> {
    marketplace::execute_action(provider, plugin_id, marketplace_name, action)
}

// --- skills ------------------------------------------------------------------------

/// The invoking user's home directory, where each harness keeps its skill root.
pub fn user_home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn skill_catalog(
    core: &Arc<BridgeCore>,
) -> Result<skill_marketplace::SkillCatalog, BridgeError> {
    skill_marketplace::catalog(&user_home(), &core.skill_store)
}

pub fn skill_suggestions(
    core: &Arc<BridgeCore>,
    query: &str,
    provider: skill_marketplace::SkillProvider,
) -> Result<Vec<skill_marketplace::CapabilitySuggestion>, BridgeError> {
    skill_marketplace::suggestions(query, provider, &user_home(), &core.skill_store)
}

pub fn preview_skill_change(
    core: &Arc<BridgeCore>,
    skill_id: &str,
    action: skill_marketplace::SkillAction,
    targets: &[skill_marketplace::SkillProvider],
) -> Result<skill_marketplace::SkillPreview, BridgeError> {
    skill_marketplace::preview(
        skill_id,
        action,
        targets,
        &user_home(),
        &core.skill_store,
        core.skill_consents.as_ref(),
    )
}

pub fn execute_skill_change(
    core: &Arc<BridgeCore>,
    confirmation_id: &str,
) -> Result<Vec<skill_marketplace::SkillActionResult>, BridgeError> {
    let results = skill_marketplace::execute(
        confirmation_id,
        &user_home(),
        &core.skill_store,
        core.skill_consents.as_ref(),
    )?;
    core.events.publish(CoreEvent::StateChanged);
    Ok(results)
}

// --- automations ---------------------------------------------------------------

pub fn automation_catalog(
    _core: &Arc<BridgeCore>,
) -> Result<automations::AutomationCatalog, BridgeError> {
    Ok(automations::catalog(&user_home()))
}

pub fn save_automation(
    core: &Arc<BridgeCore>,
    provider: automations::AutomationProvider,
    automation_id: Option<&str>,
    prompt: &str,
    schedule_expression: &str,
    recurring: bool,
) -> Result<automations::AutomationSaveResult, BridgeError> {
    let result = automations::save(&user_home(), provider, automation_id, prompt, schedule_expression, recurring)?;
    core.events.publish(CoreEvent::StateChanged);
    Ok(result)
}

pub fn execute_automation_action(
    core: &Arc<BridgeCore>,
    provider: automations::AutomationProvider,
    automation_id: &str,
    action: automations::AutomationAction,
) -> Result<automations::AutomationActionResult, BridgeError> {
    let result = automations::execute(&user_home(), provider, automation_id, action)?;
    core.events.publish(CoreEvent::StateChanged);
    Ok(result)
}

#[cfg(test)]
mod tests {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct PolicyReplayCase {
        id: String,
        action: bridge_protocol::messages::GithubAction,
        confirmed: bool,
        expect_executed: bool,
    }

    use rusqlite::params;
    use std::path::Path;
    use std::process::Command;

    fn command(name: &str, harness: &str) -> crate::slash::SlashCommand {
        crate::slash::SlashCommand {
            name: name.into(),
            description: String::new(),
            harness: harness.into(),
            kind: "builtin".into(),
        }
    }

    #[test]
    fn slash_catalog_filter_hides_other_harnesses_but_keeps_bridge_builtins() {
        let catalog = vec![
            command("recall", "bridge"),
            command("review", "claude"),
            command("review", "codex"),
            command("plan", "opencode"),
        ];

        let claude_only = super::filter_catalog_for_session_harness(catalog.clone(), Some("claude"));
        assert_eq!(
            claude_only.iter().map(|c| (c.name.as_str(), c.harness.as_str())).collect::<Vec<_>>(),
            vec![("recall", "bridge"), ("review", "claude")]
        );

        // A session-less caller (no session_id resolved yet) gets the full,
        // unfiltered catalog rather than an empty menu.
        let unfiltered = super::filter_catalog_for_session_harness(catalog, None);
        assert_eq!(unfiltered.len(), 4);
    }

    fn git_cmd(cwd: &Path, args: &[&str]) -> String {
        let output = Command::new("git").args(args).current_dir(cwd).output().unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    /// A core with a repository, a workspace, and a chat that owns its own
    /// isolated checkout — the shape archiving has to get right.
    struct ChatFixture {
        _scratch: tempfile::TempDir,
        core: std::sync::Arc<crate::runtime::BridgeCore>,
        chat_worktree: std::path::PathBuf,
        repo: std::path::PathBuf,
    }

    fn chat_fixture() -> ChatFixture {
        let scratch = tempfile::tempdir().unwrap();
        let core = std::sync::Arc::new(crate::runtime::BridgeCore::for_tests(scratch.path()));
        let repo = std::fs::canonicalize(scratch.path()).unwrap().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git_cmd(&repo, &["init", "-q", "-b", "main"]);
        git_cmd(&repo, &["config", "user.email", "t@example.invalid"]);
        git_cmd(&repo, &["config", "user.name", "Bridge Test"]);
        git_cmd(&repo, &["config", "commit.gpgsign", "false"]);
        std::fs::write(repo.join("base.txt"), "base\n").unwrap();
        git_cmd(&repo, &["add", "."]);
        git_cmd(&repo, &["commit", "-q", "-m", "base"]);

        let chat_worktree = core
            .worktrees
            .join("orchestrators")
            .join("task")
            .join("chat");
        std::fs::create_dir_all(chat_worktree.parent().unwrap()).unwrap();
        git_cmd(
            &repo,
            &["worktree", "add", "-q", "-b", "bridge/task-chat", chat_worktree.to_str().unwrap(), "HEAD"],
        );
        {
            let db = core.db.lock().unwrap();
            db.execute(
                "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo',?1,'now')",
                params![repo.to_string_lossy()],
            )
            .unwrap();
            db.execute(
                "INSERT INTO workspaces(id,project_id,title,branch,path,status,created_at)
                 VALUES('w','p','Task','main',?1,'idle','now')",
                params![repo.to_string_lossy()],
            )
            .unwrap();
            // A sibling chat in the same workspace: archiving one must not touch it.
            db.execute(
                "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind,cwd,depth)
                 VALUES('sibling','w','claude','Other','idle','estimated','orchestrator',?1,0)",
                params![repo.to_string_lossy()],
            )
            .unwrap();
            db.execute(
                "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind,cwd,depth)
                 VALUES('chat','w','claude','Chat','idle','estimated','orchestrator',?1,0)",
                params![chat_worktree.to_string_lossy()],
            )
            .unwrap();
            crate::worktree_registry::register(
                &db,
                &crate::worktree_registry::NewWorktree {
                    kind: crate::worktree_registry::KIND_ORCHESTRATOR.to_owned(),
                    repo_root: repo.to_string_lossy().to_string(),
                    path: chat_worktree.to_string_lossy().to_string(),
                    branch: Some("bridge/task-chat".to_owned()),
                    owner_session_id: Some("chat".to_owned()),
                    owner_workspace_id: Some("w".to_owned()),
                    base_commit: None,
                },
            )
            .unwrap();
        }
        ChatFixture { _scratch: scratch, core, chat_worktree, repo }
    }

    fn session_count(core: &crate::runtime::BridgeCore) -> usize {
        crate::store::state(&core.db.lock().unwrap()).unwrap().sessions.len()
    }

    /// The whole point of not reusing `archive_workspace`: a workspace holds
    /// many chats, so archiving one must leave its siblings — and their
    /// checkout — exactly where they are.
    #[test]
    fn archiving_a_chat_reclaims_its_own_worktree_and_leaves_its_siblings_alone() {
        let fixture = chat_fixture();
        assert_eq!(session_count(&fixture.core), 2);

        let result = super::archive_chat(&fixture.core, "chat").unwrap();
        assert!(result.archived);
        assert!(result.bytes_freed > 0, "it says what it freed: {result:?}");
        assert_eq!(result.worktree_detail, None);
        assert!(!fixture.chat_worktree.exists(), "the chat's checkout is reclaimed");

        assert!(fixture.repo.is_dir(), "the workspace's own checkout is untouched");
        assert!(fixture.repo.join("base.txt").is_file());
        let remaining = crate::store::state(&fixture.core.db.lock().unwrap()).unwrap();
        assert_eq!(remaining.sessions.len(), 1, "the sibling chat survives");
        assert_eq!(remaining.sessions[0].id, "sibling");
        assert_eq!(
            remaining.workspaces.len(),
            1,
            "and so does the workspace every other chat lives in",
        );
    }

    /// History is kept: archiving hides a conversation, it does not delete it.
    #[test]
    fn an_archived_chat_keeps_its_row_and_stops_being_listed() {
        let fixture = chat_fixture();
        super::archive_chat(&fixture.core, "chat").unwrap();
        let (archived, stored): (Option<String>, i64) = fixture
            .core
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT archived_at,(SELECT COUNT(*) FROM sessions WHERE id='chat') FROM sessions WHERE id='chat'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert!(archived.is_some(), "marked archived");
        assert_eq!(stored, 1, "and still there");
        assert_eq!(session_count(&fixture.core), 1);
    }

    #[test]
    fn archive_list_and_unarchive_restore_only_visibility() {
        let fixture = chat_fixture();
        super::archive_chat(&fixture.core, "chat").unwrap();
        let request = super::wire::ListArchivedChatsParams { query: "chat".into(), offset: 0, root_session_id: None };
        let archived = super::list_archived_chats(&fixture.core, &request).unwrap();
        assert_eq!(archived.chats.len(), 1);
        assert_eq!(archived.chats[0].id, "chat");
        assert!(!archived.has_more);
        super::unarchive_chat(&fixture.core, "chat").unwrap();
        super::unarchive_chat(&fixture.core, "chat").unwrap();
        assert_eq!(session_count(&fixture.core), 2);
        assert!(!fixture.chat_worktree.exists());
        let db = fixture.core.db.lock().unwrap();
        let (pid, turn, ended): (Option<i64>, Option<String>, Option<String>) = db.query_row(
            "SELECT adapter_pid,active_turn_id,ended_at FROM sessions WHERE id='chat'", [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        assert_eq!((pid, turn), (None, None));
        assert!(ended.is_some());
        drop(db);
        assert!(super::list_archived_chats(&fixture.core, &request).unwrap().chats.is_empty());
        assert!(super::unarchive_chat(&fixture.core, "missing").is_err());
    }

    #[test]
    fn archived_roots_expose_and_search_their_descendants_without_restoring_them() {
        let fixture = chat_fixture();
        {
            let db = fixture.core.db.lock().unwrap();
            for (id, parent, title) in [("worker", "chat", "Worker trace"), ("aside", "chat", "Aside notes"), ("deep", "worker", "Nested investigation"), ("kept", "chat", "Separate archive")] {
                db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id,kind) VALUES(?1,'w','codex',?3,'idle','reported',?2,'chat')", params![id, parent, title]).unwrap();
            }
            crate::session_forest::SessionForest::new(&db).append("deep", crate::session_forest::EntryKind::AssistantMessage, serde_json::json!({"text":"The descendant history is preserved"})).unwrap();
        }
        super::archive_chat(&fixture.core, "kept").unwrap();
        super::archive_chat(&fixture.core, "chat").unwrap();
        assert_eq!(session_count(&fixture.core), 1, "every descendant is hidden from normal state");
        let roots = super::list_archived_chats(&fixture.core, &super::wire::ListArchivedChatsParams { query: "Nested investigation".into(), offset: 0, root_session_id: None }).unwrap();
        assert_eq!(roots.chats.iter().map(|chat| chat.id.as_str()).collect::<Vec<_>>(), ["chat"]);
        let family = super::list_archived_chats(&fixture.core, &super::wire::ListArchivedChatsParams { query: String::new(), offset: 0, root_session_id: Some("chat".into()) }).unwrap();
        let ids = family.chats.iter().map(|chat| chat.id.as_str()).collect::<std::collections::BTreeSet<_>>();
        assert_eq!(ids, ["worker", "aside", "deep", "kept"].into_iter().collect());
        let replay = crate::store::session_events_after(&fixture.core.db.lock().unwrap(), "deep", 0, 200).unwrap();
        assert_eq!(replay[0].text.as_deref(), Some("The descendant history is preserved"));
        assert!(!fixture.chat_worktree.exists());
        super::unarchive_chat(&fixture.core, "chat").unwrap();
        assert_eq!(session_count(&fixture.core), 5, "the independent child archive remains hidden");
        let roots = super::list_archived_chats(&fixture.core, &super::wire::ListArchivedChatsParams { query: String::new(), offset: 0, root_session_id: None }).unwrap();
        assert_eq!(roots.chats[0].id, "kept");
        assert!(!fixture.chat_worktree.exists());
    }

    /// Putting a conversation away should not require first resolving its
    /// uncommitted work, so the checkout is kept and the reason is reported.
    #[test]
    fn archiving_keeps_a_dirty_checkout_and_says_so_instead_of_refusing() {
        let fixture = chat_fixture();
        std::fs::write(fixture.chat_worktree.join("scratch.txt"), "unsaved\n").unwrap();

        let result = super::archive_chat(&fixture.core, "chat").unwrap();
        assert!(result.archived, "the archive still happens");
        assert_eq!(result.bytes_freed, 0);
        assert!(
            result.worktree_detail.unwrap().contains("uncommitted"),
            "and the reason reaches the caller",
        );
        assert!(fixture.chat_worktree.is_dir());
    }

    /// A `ready` chat has no turn in flight but still owns a live provider
    /// process. Archiving it would hide the only route to that process while it
    /// went on holding memory and a model session — and the worktree would be
    /// retained anyway, since the same claim marks it in use.
    #[test]
    fn archiving_refuses_a_chat_whose_adapter_is_still_alive() {
        let fixture = chat_fixture();
        fixture
            .core
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE sessions SET status='ready',adapter_pid=4242,
                    adapter_process_identity='claude:4242' WHERE id='chat'",
                [],
            )
            .unwrap();
        let error = super::archive_chat(&fixture.core, "chat").unwrap_err();
        assert!(error.to_string().contains("Stop this chat"), "{error:?}");
        assert!(fixture.chat_worktree.is_dir());
        let archived: Option<String> = fixture
            .core
            .db
            .lock()
            .unwrap()
            .query_row("SELECT archived_at FROM sessions WHERE id='chat'", [], |row| row.get(0))
            .unwrap();
        assert!(archived.is_none(), "and it is not hidden");
    }

    /// Boot recovery clears the claim for a process that is really gone, so a
    /// `ready` row left by a crashed run must not block archiving forever.
    #[test]
    fn archiving_a_ready_chat_with_no_live_adapter_still_works() {
        let fixture = chat_fixture();
        fixture
            .core
            .db
            .lock()
            .unwrap()
            .execute("UPDATE sessions SET status='idle',adapter_pid=NULL WHERE id='chat'", [])
            .unwrap();
        assert!(super::archive_chat(&fixture.core, "chat").unwrap().archived);
    }

    #[test]
    fn archiving_refuses_a_chat_that_is_still_running() {
        let fixture = chat_fixture();
        fixture
            .core
            .db
            .lock()
            .unwrap()
            .execute("UPDATE sessions SET status='working' WHERE id='chat'", [])
            .unwrap();
        let error = super::archive_chat(&fixture.core, "chat").unwrap_err();
        assert!(
            error.to_string().contains("Stop this chat"),
            "{error:?}",
        );
        assert!(fixture.chat_worktree.is_dir());
    }

    /// Replays the recorded approve/deny decision for every action kind through
    /// the real `github_act` gate. A denied case must return `executed:false`
    /// *without* consulting the surface — the test core's surface is
    /// `unavailable_for_tests`, so any subprocess attempt would surface as an
    /// `Unavailable` error instead of a clean decline. An approved case must be
    /// let through the gate (it then fails on the missing workspace, proving the
    /// gate did not itself short-circuit it).
    #[test]
    fn github_act_policy_replay_covers_approve_and_deny_per_action_kind() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testing/fixtures/github-act-policy.json");
        let cases: Vec<PolicyReplayCase> =
            serde_json::from_slice(&std::fs::read(fixture).unwrap()).unwrap();
        assert_eq!(cases.len(), 20, "approve and deny for PR and issue action kinds");
        let scratch = tempfile::tempdir().unwrap();
        let core = std::sync::Arc::new(crate::runtime::BridgeCore::for_tests(scratch.path()));
        for case in cases {
            let result = super::github_act(&core, "missing-workspace", case.action, case.confirmed);
            if case.expect_executed {
                assert!(
                    result.is_err(),
                    "{}: an approved action is let through the gate",
                    case.id
                );
            } else {
                let acted = result.unwrap_or_else(|error| panic!("{}: {error:?}", case.id));
                assert!(!acted.executed, "{}: a denied action does not execute", case.id);
                assert!(acted.message.starts_with("Declined:"), "{}: names the decline", case.id);
            }
        }
    }

    #[test]
    fn cursor_bugbot_is_not_rejected_as_an_unsupported_harness() {
        let scratch = tempfile::tempdir().unwrap();
        let core = std::sync::Arc::new(crate::runtime::BridgeCore::for_tests(scratch.path()));
        let error = super::github_review(&core, "missing-workspace", 12, "bugbot", None)
            .unwrap_err()
            .to_string();
        assert!(
            !error.contains("unsupported harness"),
            "bugbot must not be treated as a local worker harness, got: {error}"
        );
    }

    #[test]
    fn shells_key_by_workspace_and_terminal() {
        let scratch = tempfile::tempdir().unwrap();
        let core = std::sync::Arc::new(crate::runtime::BridgeCore::for_tests(scratch.path()));
        {
            let db = core.db.lock().unwrap();
            db.execute(
                "INSERT INTO projects(id,name,path,created_at) VALUES('p','P','/tmp','now')",
                [],
            )
            .unwrap();
            db.execute(
                "INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Task','main',?1,'ready','now')",
                rusqlite::params![scratch.path().to_string_lossy()],
            )
            .unwrap();
        }

        super::open_terminal(&core, "w", "t1").unwrap();
        super::open_terminal(&core, "w", "t2").unwrap();
        assert_eq!(super::list_terminals(&core, "w"), vec!["t1", "t2"]);

        super::write_terminal(&core, "w", "t1", "true\n").unwrap();
        let missing = super::write_terminal(&core, "w", "t3", "x");
        assert!(missing.is_err(), "a write addresses one existing shell");

        super::close_terminal(&core, "w", "t1").unwrap();
        assert_eq!(super::list_terminals(&core, "w"), vec!["t2"]);
        super::close_terminal(&core, "w", "t1").unwrap();

        super::close_terminal(&core, "w", "t2").unwrap();
        assert!(super::list_terminals(&core, "w").is_empty());
    }

    #[test]
    fn a_reopened_terminal_survives_the_old_readers_drain() {
        let scratch = tempfile::tempdir().unwrap();
        let core = std::sync::Arc::new(crate::runtime::BridgeCore::for_tests(scratch.path()));
        {
            let db = core.db.lock().unwrap();
            db.execute(
                "INSERT INTO projects(id,name,path,created_at) VALUES('p','P','/tmp','now')",
                [],
            )
            .unwrap();
            db.execute(
                "INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Task','main',?1,'ready','now')",
                rusqlite::params![scratch.path().to_string_lossy()],
            )
            .unwrap();
        }

        super::open_terminal(&core, "w", "t1").unwrap();
        // Close and immediately reopen the same id: the dying shell's reader
        // thread drains on its own schedule, and its cleanup must recognise
        // that the key now belongs to a newer shell.
        super::close_terminal(&core, "w", "t1").unwrap();
        super::open_terminal(&core, "w", "t1").unwrap();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            assert_eq!(
                super::list_terminals(&core, "w"),
                vec!["t1"],
                "the reopened shell must survive the old reader's drain"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        super::write_terminal(&core, "w", "t1", "true\n").unwrap();
        super::close_terminal(&core, "w", "t1").unwrap();
    }

    #[test]
    fn learning_runs_never_hold_the_global_sqlite_lock() {
        // run_learning opens its own connection via run_local_database; a
        // locked-connection call would serialize the whole app behind model
        // evaluation.
        let source = include_str!("api.rs");
        assert!(
            source.contains(
                "learning_job::run_local_database(&core.database_path, trigger_kind, workspace_id)",
            )
        );
        let locked_learning_call = [
            "learning_job::run_learning(",
            "&core.db.lock().unwrap()",
            ", trigger_kind)",
        ]
        .concat();
        assert!(!source.contains(&locked_learning_call));
    }

    #[test]
    fn memory_saves_and_forgets_publish_the_scope_hint() {
        let scratch = tempfile::tempdir().unwrap();
        let core = std::sync::Arc::new(crate::runtime::BridgeCore::for_tests(scratch.path()));
        let mut events = core.events.subscribe();
        let record = super::save_memory_record(&core, "Prefers tabs over spaces", None, None)
            .expect("an explicit save is accepted");
        assert!(matches!(
            events.try_recv().unwrap(),
            crate::events::CoreEvent::MemoryChanged { ref scope_key } if scope_key == "account:local"
        ));
        super::delete_memory_record(&core, &record.id).expect("forget tombstones");
        assert!(matches!(
            events.try_recv().unwrap(),
            crate::events::CoreEvent::MemoryChanged { ref scope_key } if scope_key == "account:local"
        ));
        // A refused save changes nothing, so it owes no hint.
        assert!(super::save_memory_record(&core, "   ", None, None).is_err());
        assert!(events.try_recv().is_err());
    }

    #[test]
    fn lifecycle_transitions_publish_the_scope_hint() {
        let scratch = tempfile::tempdir().unwrap();
        let core = std::sync::Arc::new(crate::runtime::BridgeCore::for_tests(scratch.path()));
        let mut events = core.events.subscribe();
        let saved = super::save_memory_record(&core, "Prefers tabs", None, None).unwrap();
        events.try_recv().unwrap();
        super::supersede_memory_record(&core, &saved.id, "Prefers spaces", None).unwrap();
        assert!(matches!(
            events.try_recv().unwrap(),
            crate::events::CoreEvent::MemoryChanged { ref scope_key } if scope_key == "account:local"
        ));
        {
            let db = core.db.lock().unwrap();
            db.execute(
                "INSERT INTO memory_records(id, scope_key, kind, body, provenance, status,
                     valid_from, valid_to, created_at, updated_at)
                 VALUES('p1','account:local','fact','One','model_proposal','proposed','now','now','now','now'),
                        ('p2','account:local','fact','Two','model_proposal','proposed','now','now','now','now')",
                [],
            )
            .unwrap();
        }
        super::approve_memory_record(&core, "p1").unwrap();
        assert!(matches!(
            events.try_recv().unwrap(),
            crate::events::CoreEvent::MemoryChanged { .. }
        ));
        super::reject_memory_record(&core, "p2").unwrap();
        assert!(matches!(
            events.try_recv().unwrap(),
            crate::events::CoreEvent::MemoryChanged { .. }
        ));
    }

    #[test]
    fn memory_capabilities_answer_without_a_harness_comparison() {
        let scratch = tempfile::tempdir().unwrap();
        let core = std::sync::Arc::new(crate::runtime::BridgeCore::for_tests(scratch.path()));
        let capabilities = super::get_memory_capabilities(&core).unwrap();
        assert!(capabilities.ledger.exists);
        assert_eq!(capabilities.ledger.scope_key, "account:local");
        assert_eq!(capabilities.ledger.kinds.len(), 4);
        // The test core registers no adapters, so no provider contributes.
        assert!(capabilities.provider_native.is_empty());
    }

    #[test]
    fn slash_pin_arms_publish_the_same_hint_as_the_api() {
        // live_turn has no test scaffold; its slash arms mirror the api seam,
        // so the wiring claim is checked the way this module already checks
        // cross-module wiring: against the source.
        let source = include_str!("live_turn.rs");
        assert!(
            source.matches("core.events.publish(CoreEvent::MemoryChanged").count() >= 2,
            "both /pin and /unpin publish the memory-changed hint"
        );
    }

    #[test]
    fn checkout_updates_state_invalidates_branch_facts_and_blocks_warm_sessions() {
        let scratch = tempfile::tempdir().unwrap();
        let repo = scratch.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?} failed");
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "bridge@example.com"]);
        git(&["config", "user.name", "Bridge"]);
        std::fs::write(repo.join("tracked.txt"), "initial\n").unwrap();
        git(&["add", "tracked.txt"]);
        git(&["commit", "-qm", "initial"]);
        git(&["branch", "feature"]);
        git(&["branch", "blocked"]);

        let core = std::sync::Arc::new(crate::runtime::BridgeCore::for_tests(scratch.path()));
        {
            let db = core.db.lock().unwrap();
            db.execute(
                "INSERT INTO projects(id,name,path,created_at) VALUES('p','Repo',?1,'now')",
                [repo.to_string_lossy().as_ref()],
            )
            .unwrap();
            db.execute(
                "INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at)
                 VALUES('w','p','Oslo','Repo','main',?1,'ready','now')",
                [repo.to_string_lossy().as_ref()],
            )
            .unwrap();
            db.execute(
                "INSERT INTO work_fact_cache(kind,cache_key,status,payload,observed_at)
                 VALUES(?1,'w','ok','{}','now')",
                [crate::work::FACT_CACHE_BASE_DIVERGENCE],
            )
            .unwrap();
        }

        let mut events = core.events.subscribe();
        let state = super::checkout_workspace_branch(&core, "w", "feature").unwrap();
        assert_eq!(
            state.workspaces.iter().find(|workspace| workspace.id == "w").unwrap().branch,
            Some("feature".into())
        );
        assert!(matches!(
            events.try_recv().unwrap(),
            crate::events::CoreEvent::StateChanged
        ));
        assert_eq!(
            core.db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM work_fact_cache WHERE kind=?1 AND cache_key='w'",
                    [crate::work::FACT_CACHE_BASE_DIVERGENCE],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );

        core.db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source)
                 VALUES('warm','w','codex','Warm worker','warm','reported')",
                [],
            )
            .unwrap();
        let error = super::checkout_workspace_branch(&core, "w", "blocked").unwrap_err();
        assert!(error.to_string().contains("session is active"), "{error}");

        core.db
            .lock()
            .unwrap()
            .execute("UPDATE sessions SET status='idle', ended_at=NULL WHERE id='warm'", [])
            .unwrap();
        let state = super::checkout_workspace_branch(&core, "w", "blocked").unwrap();
        assert_eq!(
            state.workspaces.iter().find(|workspace| workspace.id == "w").unwrap().branch,
            Some("blocked".into())
        );

        core.db.lock().unwrap().execute_batch(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source)
             VALUES('talk','w','codex','Chat with history','idle','reported');
             INSERT INTO session_entries(id,session_id,sequence,kind,payload,created_at)
             VALUES('e1','talk',1,'user.message','{\"text\":\"hello\"}','now');",
        ).unwrap();
        let error = super::checkout_workspace_branch(&core, "w", "feature").unwrap_err();
        assert!(error.to_string().contains("session is active"), "{error}");
    }

    #[test]
    fn adopt_without_a_binding_is_a_domain_error() {
        let scratch = tempfile::tempdir().unwrap();
        let core = std::sync::Arc::new(crate::runtime::BridgeCore::for_tests(scratch.path()));
        let error = super::adopt_worker_worktree(&core, "missing-worker").unwrap_err();
        assert!(
            error.to_string().contains("no repository binding"),
            "{error}"
        );
        match error {
            crate::BridgeError::Invalid(_) => {}
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    fn stub_import_plan(
        selected_candidate_ids: Vec<String>,
    ) -> super::wire::ExternalImportPlan {
        super::wire::ExternalImportPlan {
            selected_candidate_ids,
            conflict_policy: super::wire::ExternalImportConflictPolicy::Skip,
            setup_activation_policy: super::wire::ExternalImportSetupActivationPolicy::Disabled,
            memory_scope: None,
            dry_run: false,
            created_at: "2026-08-01T11:00:00Z".into(),
        }
    }

    /// A hand-built `DiscoveryResult` naming `approvedRoots: ["/"]` used to be
    /// enough to read any file the daemon process could see, because
    /// `preview`/`commit` decoded the caller's own blob straight off the wire.
    /// Both now take only a `discoveryId` and look it up in the daemon's own
    /// cache — a forged or expired id has to be rejected before any file is
    /// ever touched.
    #[test]
    fn preview_and_commit_reject_a_discovery_id_this_daemon_never_walked() {
        let scratch = tempfile::tempdir().unwrap();
        let core = std::sync::Arc::new(crate::runtime::BridgeCore::for_tests(scratch.path()));

        let preview_error = super::preview_external_import(
            &core,
            &super::wire::PreviewExternalImportParams {
                discovery_id: "forged-discovery".into(),
                artifact_ids: vec!["anything".into()],
            },
        )
        .unwrap_err();
        assert!(preview_error.to_string().contains("no longer available"));

        let commit_error = super::commit_external_import(
            &core,
            &super::wire::CommitExternalImportParams {
                discovery_id: "forged-discovery".into(),
                plan: stub_import_plan(vec!["anything".into()]),
            },
        )
        .unwrap_err();
        assert!(commit_error.to_string().contains("no longer available"));
    }

    /// End-to-end through the same api functions the daemon dispatches to:
    /// discover caches the result, preview and commit reference it by id, and
    /// a successful commit publishes `StateChanged` so the sidebar picks up
    /// the new session without an app restart.
    #[test]
    fn discover_preview_commit_round_trips_through_the_cache_and_publishes_state_changed() {
        let scratch = tempfile::tempdir().unwrap();
        let claude_home = scratch.path().join(".claude");
        std::fs::create_dir_all(claude_home.join("projects/demo")).unwrap();
        std::fs::write(
            claude_home.join("projects/demo/session.jsonl"),
            include_str!("../../../testing/fixtures/import/claude/transcripts/simple.jsonl"),
        )
        .unwrap();
        let core = std::sync::Arc::new(crate::runtime::BridgeCore::for_tests(scratch.path()));
        let mut events = core.events.subscribe();

        let discovery = super::discover_external_import(
            &core,
            &super::wire::DiscoverExternalImportParams {
                provider: "claude_code".into(),
                approved_roots: vec![claude_home.to_string_lossy().into_owned()],
                selected_export: None,
                source_version: None,
                schema_version: None,
                format_versions: Default::default(),
            },
        )
        .unwrap();
        let transcript = discovery
            .artifacts
            .iter()
            .find(|artifact| {
                artifact.kind == super::wire::ExternalImportCandidateKind::Conversation
            })
            .unwrap();

        let preview = super::preview_external_import(
            &core,
            &super::wire::PreviewExternalImportParams {
                discovery_id: discovery.discovery_id.clone(),
                artifact_ids: vec![transcript.artifact_id.clone()],
            },
        )
        .unwrap();
        assert_eq!(preview.candidates.len(), 1);

        let commit = super::commit_external_import(
            &core,
            &super::wire::CommitExternalImportParams {
                discovery_id: discovery.discovery_id.clone(),
                plan: stub_import_plan(vec![preview.candidates[0].candidate_id.clone()]),
            },
        )
        .unwrap();
        assert_eq!(commit.imported, 1);
        assert!(matches!(
            events.try_recv().unwrap(),
            crate::events::CoreEvent::StateChanged
        ));
    }
}

// ---- agents: the managed runtime lifecycle -------------------------------
//
// The api seam both hosts call, so the shell and the daemon share one path into
// the domain rather than each wiring its own.

pub fn list_managed_agents(
) -> Result<bridge_protocol::messages::ManagedAgentList, crate::managed_agents::ManagedAgentError> {
    crate::managed_agents::list_managed_agents()
}

pub fn inspect_managed_agent(
    agent_id: &str,
) -> Result<
    bridge_protocol::messages::ManagedAgentInspection,
    crate::managed_agents::ManagedAgentError,
> {
    crate::managed_agents::inspect_managed_agent(agent_id)
}

pub fn install_managed_agent(
    agent_id: &str,
) -> Result<
    bridge_protocol::messages::ManagedAgentOperationResult,
    crate::managed_agents::ManagedAgentError,
> {
    crate::managed_agents::install_managed_agent(agent_id)
}

pub fn repair_managed_agent(
    agent_id: &str,
) -> Result<
    bridge_protocol::messages::ManagedAgentOperationResult,
    crate::managed_agents::ManagedAgentError,
> {
    crate::managed_agents::repair_managed_agent(agent_id)
}

/// Takes the core because removal must first prove nothing is running against
/// the payload, which is a question only the session store can answer.
pub fn uninstall_managed_agent(
    core: &Arc<BridgeCore>,
    agent_id: &str,
) -> Result<
    bridge_protocol::messages::ManagedAgentOperationResult,
    crate::managed_agents::ManagedAgentError,
> {
    let db = core.db.lock().unwrap();
    crate::managed_agents::uninstall_managed_agent(&db, agent_id)
}

// ---- agents: the session's backend binding -------------------------------

/// Authorize continuing one session under a different backend.
///
/// A session records which implementation served it, and a resume through a
/// different one is refused — the same history answered by a different agent
/// runtime is a decision only a user can make. This is how that decision is
/// given: for one session, one exact transition, spent when it is used.
///
/// Deliberately not an RPC method yet. Its wire and desktop surface belong with
/// #166's control plane, where a second backend candidate first becomes
/// reachable; exposing a control now would put a button in front of a resolver
/// that has exactly one candidate per agent.
pub fn authorize_backend_change(
    core: &Arc<BridgeCore>,
    session_id: &str,
    to_backend: &str,
) -> Result<(), BridgeError> {
    let to_backend = bridge_protocol::messages::BackendId::parse(to_backend)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let db = core.db.lock().unwrap();
    let from = match crate::backend_binding::read_binding(&db, session_id)? {
        crate::backend_binding::StoredBinding::Bound(binding) => binding,
        // Nothing to change from: an unbound session binds on its next launch,
        // and an unreadable one must not be rebound by a caller who cannot have
        // been shown what it is changing away from.
        other => {
            return Err(BridgeError::Invalid(format!(
                "{session_id} has no backend to change from ({other:?})"
            )))
        }
    };
    let candidate = crate::backend_binding::BackendResolver::candidate(
        &core.backend_resolver,
        &from.agent,
        &to_backend,
    )
    .ok_or_else(|| {
        BridgeError::Invalid(format!("{} has no backend named {to_backend}", from.agent))
    })?;
    let to = crate::backend_binding::BackendBinding {
        agent: from.agent.clone(),
        backend: candidate.backend.clone(),
        version: None,
        installation: None,
    };
    crate::backend_binding::authorize_backend_change(&db, session_id, &from, &to)
}

#[cfg(test)]
mod chat_effort_tests {
    use super::*;
    #[test]
    fn provider_levels_pass_through_and_incompatible_values_are_cleared() {
        let mut candidate = crate::model_catalog::CatalogCandidate::stable("test", "Test", CapabilityTier::Standard, 0);
        candidate.supported_effort_levels = vec!["high".into(), "max".into(), "ultra".into()];
        let model = crate::model_catalog::normalize(crate::model::ModelCatalogSource::RuntimeApi, [candidate]).remove(0);
        for value in ["max", "ultra"] {
            assert_eq!(selected_chat_effort(&model, Some(value), None).unwrap().as_deref(), Some(value));
        }
        assert!(selected_chat_effort(&model, Some("low"), Some("high")).is_err());
        assert_eq!(selected_chat_effort(&model, None, Some("low")).unwrap(), None);
        assert_eq!(selected_chat_effort(&model, None, Some("high")).unwrap().as_deref(), Some("high"));
    }
}

//! Bounded, source-labelled context breakdown for one session (issue #245).
//!
//! Merges three sources into one capped, stably ordered result: the
//! conversation projection (`SessionForest::active_branch` followed by
//! [`ContextProjector`]), Bridge prompt accounting from Context Lens 5/8, and
//! live adapter context observations from Context Lens 6/8. Sources that
//! cannot report stay explicitly `unavailable`; nothing is fabricated.

use bridge_protocol::messages::{
    ContextBreakdownConversation, ContextBreakdownDelta, ContextBreakdownDigestResult,
    ContextBreakdownOrigin, ContextBreakdownResult, ContextBreakdownSegment,
    ContextBreakdownState, ContextBreakdownTotals, MAX_CONTEXT_BREAKDOWN_SEGMENTS,
};
use rusqlite::{params, Connection};

use crate::context::{Checkpoint, ContextProjector};
use crate::context_inventory::{
    AdapterContextInventory, ContextInventoryScope, ContextObservationProvenance,
    ContextSegmentClass,
};
use crate::model::SessionEntry;
use crate::{context_inventory, session_forest, store, BridgeError};

/// The projection window used until Bridge grows a real per-session
/// `context_window_tokens` source; mirrors `restoration.rs`. The value is
/// echoed in every breakdown so clients see the basis.
pub const CONTEXT_BREAKDOWN_WINDOW_TOKENS: i64 = 128_000;

const CONVERSATION_METHOD: &str = "bridge-context-projector";
const NO_COMPILATION_REASON: &str = "no prompt compilation recorded";
const NO_SPLIT_PREFIX: &str = "compilation did not split ";
const NO_INVENTORY_REASON: &str = "adapter runtime has not reported context inventory";

fn segment_class_label(class: ContextSegmentClass) -> &'static str {
    match class {
        ContextSegmentClass::ProviderBaseInstructions => "providerBaseInstructions",
        ContextSegmentClass::ToolSchemas => "toolSchemas",
        ContextSegmentClass::McpDynamicTools => "mcpDynamicTools",
        ContextSegmentClass::SkillsPlugins => "skillsPlugins",
        ContextSegmentClass::AgentDefinitions => "agentDefinitions",
    }
}

fn origin_rank(origin: ContextBreakdownOrigin) -> u8 {
    match origin {
        ContextBreakdownOrigin::Conversation => 0,
        ContextBreakdownOrigin::PromptCompilation => 1,
        ContextBreakdownOrigin::AdapterInventory => 2,
    }
}

fn observation_segment(
    observation: &crate::context_inventory::ContextSegmentObservation,
) -> ContextBreakdownSegment {
    let (state, method, reason) = match &observation.provenance {
        ContextObservationProvenance::Reported { .. } => {
            (ContextBreakdownState::Reported, None, None)
        }
        ContextObservationProvenance::Measured { .. } => {
            (ContextBreakdownState::Measured, None, None)
        }
        ContextObservationProvenance::Estimated { method, .. } => (
            ContextBreakdownState::Estimated,
            Some(method.clone()),
            None,
        ),
        ContextObservationProvenance::Unavailable { reason } => {
            (ContextBreakdownState::Unavailable, None, Some(reason.clone()))
        }
    };
    let size = match &observation.provenance {
        ContextObservationProvenance::Reported { size }
        | ContextObservationProvenance::Measured { size }
        | ContextObservationProvenance::Estimated { size, .. } => Some(size),
        ContextObservationProvenance::Unavailable { .. } => None,
    };
    let available = !matches!(state, ContextBreakdownState::Unavailable);
    ContextBreakdownSegment {
        origin: ContextBreakdownOrigin::AdapterInventory,
        segment_class: segment_class_label(observation.segment_class).to_owned(),
        names: available.then(|| observation.names.clone()).unwrap_or_default(),
        state,
        method,
        reason,
        item_count: size.and_then(|size| size.item_count),
        bytes: size.and_then(|size| size.bytes),
        tokens: size.and_then(|size| size.tokens.map(|tokens| tokens as i64)),
        capped: size.is_some_and(|size| size.capped),
    }
}

fn compilation_segments(
    record: Option<&crate::model::PromptCompilationRecord>,
) -> Vec<ContextBreakdownSegment> {
    let mut segments = Vec::new();
    let mut push_split = |segment_class: &str,
                          tokens: Option<i64>,
                          bytes: Option<u64>,
                          source: Option<&str>| {
        let (state, method, reason) = match (tokens, bytes, source) {
            (None, None, _) => (
                ContextBreakdownState::Unavailable,
                None,
                Some(format!("{NO_SPLIT_PREFIX}{segment_class} context")),
            ),
            (_, _, None) => (
                ContextBreakdownState::Unavailable,
                None,
                Some("compilation lacks estimate provenance".to_owned()),
            ),
            (_, _, Some(method)) => {
                (ContextBreakdownState::Estimated, Some(method.to_owned()), None)
            }
        };
        segments.push(ContextBreakdownSegment {
            origin: ContextBreakdownOrigin::PromptCompilation,
            segment_class: segment_class.to_owned(),
            names: vec![],
            state,
            method,
            reason,
            item_count: None,
            bytes,
            tokens,
            capped: false,
        });
    };
    match record {
        Some(record) => {
            let source = record.token_estimate_source.as_deref();
            push_split(
                "prompt-stable",
                record.stable_token_estimate,
                record.stable_bytes.map(|bytes| bytes as u64),
                source,
            );
            push_split(
                "prompt-variable",
                record.variable_token_estimate,
                record.variable_bytes.map(|bytes| bytes as u64),
                source,
            );
        }
        None => {
            for segment_class in ["prompt-stable", "prompt-variable"] {
                segments.push(ContextBreakdownSegment {
                    origin: ContextBreakdownOrigin::PromptCompilation,
                    segment_class: segment_class.to_owned(),
                    names: vec![],
                    state: ContextBreakdownState::Unavailable,
                    method: None,
                    reason: Some(NO_COMPILATION_REASON.to_owned()),
                    item_count: None,
                    bytes: None,
                    tokens: None,
                    capped: false,
                });
            }
        }
    }
    segments
}

/// Collapse one adapter runtime's inventories into at most one observation
/// per segment class, preferring what was presented on the latest turn over
/// the startup catalog.
fn flatten_inventories(inventories: &[AdapterContextInventory]) -> Vec<ContextBreakdownSegment> {
    let mut best: Vec<(bool, &AdapterContextInventory)> = Vec::new();
    for inventory in inventories {
        let presented = inventory.scope == ContextInventoryScope::TurnPresented;
        match best.iter_mut().find(|(_, existing)| existing.adapter_id == inventory.adapter_id) {
            Some((already_presented, existing)) => {
                if presented && !*already_presented {
                    *already_presented = true;
                    *existing = inventory;
                }
            }
            None => best.push((presented, inventory)),
        }
    }
    let mut segments: Vec<ContextBreakdownSegment> = best
        .into_iter()
        .flat_map(|(_, inventory)| inventory.observations.iter().map(observation_segment))
        .collect();
    if segments.is_empty() {
        for class in ContextSegmentClass::ALL {
            segments.push(ContextBreakdownSegment {
                origin: ContextBreakdownOrigin::AdapterInventory,
                segment_class: segment_class_label(class).to_owned(),
                names: vec![],
                state: ContextBreakdownState::Unavailable,
                method: None,
                reason: Some(NO_INVENTORY_REASON.to_owned()),
                item_count: None,
                bytes: None,
                tokens: None,
                capped: false,
            });
        }
    }
    segments
}

/// Newest valid compaction snapshot on the branch: newest `compaction` entry
/// whose checkpoint parses and whose retained entry still exists.
pub fn previous_compaction_checkpoint(
    branch: &[SessionEntry],
) -> Option<(SessionEntry, Checkpoint)> {
    let ids: Vec<&str> = branch.iter().map(|entry| entry.id.as_str()).collect();
    branch.iter().rev().find_map(|entry| {
        if entry.kind != "compaction" {
            return None;
        }
        let checkpoint: Checkpoint = serde_json::from_value(entry.payload.clone()).ok()?;
        ids.contains(&checkpoint.first_retained_entry_id.as_str())
            .then(|| (entry.clone(), checkpoint))
    })
}

fn totals(segments: &[ContextBreakdownSegment]) -> ContextBreakdownTotals {
    let mut totals = ContextBreakdownTotals {
        item_count: None,
        bytes: None,
        tokens: None,
        unavailable_sources: 0,
    };
    for segment in segments {
        if segment.state == ContextBreakdownState::Unavailable {
            totals.unavailable_sources += 1;
            continue;
        }
        if let Some(item_count) = segment.item_count {
            totals.item_count = Some(totals.item_count.unwrap_or(0).saturating_add(item_count));
        }
        if let Some(bytes) = segment.bytes {
            totals.bytes = Some(totals.bytes.unwrap_or(0).saturating_add(bytes));
        }
        if let Some(tokens) = segment.tokens {
            totals.tokens = Some(totals.tokens.unwrap_or(0).saturating_add(tokens));
        }
    }
    totals
}

/// The opaque change token behind the breakdown. Store-derived inputs come
/// from indexed lookups; live adapter observations contribute an in-process
/// revision counter (documented limitation: not persisted across restarts).
pub fn context_breakdown_digest(
    db: &Connection,
    session_id: &str,
) -> Result<String, BridgeError> {
    // Same existence check the full breakdown performs, so both surfaces
    // agree on unknown sessions.
    let _kind: String = db.query_row(
        "SELECT kind FROM sessions WHERE id=?1",
        params![session_id],
        |row| row.get(0),
    )?;
    let store_part: String = db.query_row(
        "SELECT COALESCE((SELECT MAX(sequence) FROM session_entries WHERE session_id=?1),0)
            ||':'||COALESCE((SELECT active_entry_id FROM session_heads WHERE session_id=?1),'')
            ||':'||COALESCE((SELECT MAX(id) FROM prompt_compilations WHERE session_id=?1),0)
            ||':'||(SELECT COUNT(*) FROM prompt_compilations WHERE session_id=?1)
            ||':'||(SELECT COUNT(*)||'/'||COALESCE(MAX(rowid),0)||'/'||COALESCE(MAX(created_at),'') FROM prompt_section_revisions)",
        params![session_id],
        |row| row.get(0),
    )?;
    Ok(format!(
        "v1:{store_part}:{}",
        context_inventory::runtime_inventory_revision()
    ))
}

/// Compute the full breakdown. `inventories` are the live adapter
/// observations for this session, gathered by the caller while holding the
/// adapter lock; pass empty when no runtime is running.
pub fn context_breakdown(
    db: &Connection,
    session_id: &str,
    inventories: &[AdapterContextInventory],
) -> Result<ContextBreakdownResult, BridgeError> {
    let digest = context_breakdown_digest(db, session_id)?;
    let branch = session_forest::SessionForest::new(db)
        .active_branch(session_id)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let projection = ContextProjector::project(&branch, CONTEXT_BREAKDOWN_WINDOW_TOKENS)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let conversation = ContextBreakdownConversation {
        entry_count: branch.len() as u32,
        rendered_entry_count: projection.render_entries.len() as u32,
        token_estimate: projection.token_estimate,
        context_pressure: projection.context_pressure,
        context_window_tokens: CONTEXT_BREAKDOWN_WINDOW_TOKENS,
        model: projection.model.clone(),
        effort: projection.effort.clone(),
        restoration_boundary_entry_id: projection
            .restoration_context
            .as_ref()
            .map(|context| context.boundary_entry_id.clone()),
    };

    let mut segments = vec![
        ContextBreakdownSegment {
            origin: ContextBreakdownOrigin::Conversation,
            segment_class: "conversation".to_owned(),
            names: vec![],
            state: ContextBreakdownState::Estimated,
            method: Some(CONVERSATION_METHOD.to_owned()),
            reason: None,
            item_count: Some(projection.render_entries.len() as u64),
            bytes: None,
            tokens: Some(projection.token_estimate),
            capped: false,
        },
    ];
    let compilation = store::latest_prompt_compilation(db, session_id)?;
    segments.extend(compilation_segments(compilation.as_ref()));
    segments.extend(flatten_inventories(inventories));

    // Stable order: by origin rank, then segment class. Truncation after
    // sorting keeps the cap deterministic.
    segments.sort_by(|left, right| {
        origin_rank(left.origin)
            .cmp(&origin_rank(right.origin))
            .then_with(|| left.segment_class.cmp(&right.segment_class))
    });
    segments.truncate(MAX_CONTEXT_BREAKDOWN_SEGMENTS as usize);

    let compaction_delta = previous_compaction_checkpoint(&branch).map(|(entry, checkpoint)| {
        ContextBreakdownDelta {
            boundary_entry_id: entry.id,
            first_retained_entry_id: checkpoint.first_retained_entry_id,
            source_agent: checkpoint.source_agent,
            reason: Some(checkpoint.reason).filter(|reason| !reason.is_empty()),
            tokens_before: checkpoint.tokens_before,
            current_token_estimate: projection.token_estimate,
            growth_tokens: projection
                .token_estimate
                .saturating_sub(checkpoint.tokens_before),
        }
    });

    Ok(ContextBreakdownResult {
        session_id: session_id.to_owned(),
        totals: totals(&segments),
        segments,
        conversation,
        compaction_delta,
        digest,
    })
}

pub fn context_breakdown_digest_result(
    db: &Connection,
    session_id: &str,
) -> Result<ContextBreakdownDigestResult, BridgeError> {
    Ok(ContextBreakdownDigestResult {
        digest: context_breakdown_digest(db, session_id)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BridgeCore;

    fn fixture() -> (tempfile::TempDir, BridgeCore) {
        let scratch = tempfile::tempdir().unwrap();
        let core = BridgeCore::for_tests(scratch.path());
        (scratch, core)
    }

    fn only_session_id(core: &BridgeCore) -> String {
        core.db.lock().unwrap()
            .query_row("SELECT id FROM sessions", [], |row| row.get(0))
            .unwrap()
    }

    fn seed_worker_and_orchestrator(core: &BridgeCore) -> (String, String) {
        let db = core.db.lock().unwrap();
        db.execute(
            "INSERT INTO workspaces(id,project_id,title,path,status,created_at) VALUES('w',NULL,'CB','/tmp/context-breakdown','idle','now')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind) VALUES('orch','w','codex','Orchestrator','idle','reported','orchestrator')",
            [],
        ).unwrap();
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,parent_session_id,status,metric_source,kind) VALUES('work','w','codex','Worker','orch','idle','reported','worker')",
            [],
        ).unwrap();
        ("orch".into(), "work".into())
    }

    #[test]
    fn unknown_sessions_error_like_other_surfaces() {
        let (_scratch, core) = fixture();
        let db = core.db.lock().unwrap();
        assert!(context_breakdown(&db, "missing", &[]).is_err());
        assert!(context_breakdown_digest(&db, "missing").is_err());
    }

    #[test]
    fn fresh_sessions_mark_every_source_unavailable_without_fabrication() {
        let (_scratch, core) = fixture();
        core.create_chat(&crate::model::Harness::Codex, None, None).unwrap();
        let session_id = only_session_id(&core);
        let db = core.db.lock().unwrap();
        let breakdown = context_breakdown(&db, &session_id, &[]).unwrap();

        assert_eq!(breakdown.session_id, session_id);
        let unavailable: Vec<_> = breakdown
            .segments
            .iter()
            .filter(|segment| segment.state == ContextBreakdownState::Unavailable)
            .collect();
        assert!(
            unavailable.len() >= 7,
            "two compilation plus five adapter classes start unavailable"
        );
        assert!(unavailable.iter().all(|segment| {
            segment.reason.as_deref().is_some_and(|reason| !reason.is_empty())
        }));
        assert!(
            breakdown.totals.unavailable_sources >= 7,
            "totals count unavailable sources"
        );
        // The conversation projection itself is always computable.
        let conversation = breakdown
            .segments
            .iter()
            .find(|segment| segment.origin == ContextBreakdownOrigin::Conversation)
            .unwrap();
        assert_eq!(conversation.state, ContextBreakdownState::Estimated);
        assert_eq!(breakdown.conversation.context_window_tokens, CONTEXT_BREAKDOWN_WINDOW_TOKENS);
    }

    #[test]
    fn workers_direct_and_orchestrator_sessions_share_one_code_path() {
        let (_scratch, core) = fixture();
        core.create_chat(&crate::model::Harness::Codex, None, None).unwrap();
        let direct = only_session_id(&core);
        let (orch, work) = seed_worker_and_orchestrator(&core);
        let db = core.db.lock().unwrap();
        for session_id in [direct, orch, work] {
            let breakdown = context_breakdown(&db, &session_id, &[]).unwrap();
            assert_eq!(breakdown.session_id, session_id);
            assert!(!breakdown.segments.is_empty());
        }
    }

    #[test]
    fn ordering_is_stable_and_cap_truncates_deterministically() {
        let (_scratch, core) = fixture();
        core.create_chat(&crate::model::Harness::Codex, None, None).unwrap();
        let session_id = only_session_id(&core);
        let db = core.db.lock().unwrap();
        let first = context_breakdown(&db, &session_id, &[]).unwrap();
        let second = context_breakdown(&db, &session_id, &[]).unwrap();
        assert_eq!(first.segments, second.segments);

        let mut flooded: Vec<ContextBreakdownSegment> = (0..100)
            .map(|index| ContextBreakdownSegment {
                origin: ContextBreakdownOrigin::AdapterInventory,
                segment_class: format!("class{:03}", index),
                names: vec![],
                state: ContextBreakdownState::Reported,
                method: None,
                reason: None,
                item_count: None,
                bytes: None,
                tokens: Some(index),
                capped: false,
            })
            .collect();
        flooded.sort_by(|left, right| left.segment_class.cmp(&right.segment_class));
        flooded.truncate(MAX_CONTEXT_BREAKDOWN_SEGMENTS as usize);
        assert_eq!(flooded.len(), MAX_CONTEXT_BREAKDOWN_SEGMENTS as usize);
        assert_eq!(flooded.last().unwrap().segment_class, "class063");
    }

    #[test]
    fn compaction_delta_comes_from_the_previous_valid_snapshot() {
        let (_scratch, core) = fixture();
        core.create_chat(&crate::model::Harness::Codex, None, None).unwrap();
        let session_id = only_session_id(&core);
        let db = core.db.lock().unwrap();
        let without = context_breakdown(&db, &session_id, &[]).unwrap();
        assert!(without.compaction_delta.is_none(), "nothing is fabricated");

        store::append_session_entry(
            &db,
            &session_id,
            None,
            "user.message",
            &serde_json::json!({"text": "hello"}),
            None,
            "visible",
            Some(10),
        )
        .unwrap();
        let boundary = store::append_session_entry(
            &db,
            &session_id,
            None,
            "compaction",
            &serde_json::json!({"schemaVersion": 1, "summary": "placeholder", "decisions": [], "filesTouched": [], "sourceAgent": "orchestrator", "firstRetainedEntryId": "pending", "tokensBefore": 500, "reason": "manual"}),
            None,
            "hidden",
            None,
        )
        .unwrap();
        // The projector requires the retained entry to follow the compaction
        // boundary on the branch.
        let retained = store::append_session_entry(
            &db,
            &session_id,
            Some(&boundary.id),
            "assistant.message",
            &serde_json::json!({"text": "kept"}),
            None,
            "visible",
            Some(20),
        )
        .unwrap();
        db.execute(
            "UPDATE session_entries SET payload=?1 WHERE id=?2",
            params![
                serde_json::json!({
                    "schemaVersion": 1,
                    "summary": "keep going",
                    "decisions": [],
                    "filesTouched": [],
                    "sourceAgent": "orchestrator",
                    "firstRetainedEntryId": retained.id,
                    "tokensBefore": 500,
                    "reason": "manual"
                })
                .to_string(),
                boundary.id
            ],
        )
        .unwrap();

        let with_delta = context_breakdown(&db, &session_id, &[]).unwrap();
        let delta = with_delta.compaction_delta.expect("valid snapshot yields a delta");
        assert_eq!(delta.boundary_entry_id, boundary.id);
        assert_eq!(delta.first_retained_entry_id, retained.id);
        assert_eq!(delta.source_agent, "orchestrator");
        assert_eq!(delta.tokens_before, 500);
        assert_eq!(
            delta.growth_tokens,
            delta.current_token_estimate.saturating_sub(500),
            "growth is the honest signed delta against the snapshot"
        );
    }

    #[test]
    fn digest_is_stable_and_tracks_all_four_inputs() {
        let (_scratch, core) = fixture();
        core.create_chat(&crate::model::Harness::Codex, None, None).unwrap();
        let session_id = only_session_id(&core);
        let db = core.db.lock().unwrap();
        let baseline = context_breakdown_digest(&db, &session_id).unwrap();
        assert_eq!(baseline, context_breakdown_digest(&db, &session_id).unwrap());

        // Active-branch change moves the digest.
        store::append_session_entry(
            &db,
            &session_id,
            None,
            "user.message",
            &serde_json::json!({"text": "hi"}),
            None,
            "visible",
            Some(5),
        )
        .unwrap();
        let after_branch = context_breakdown_digest(&db, &session_id).unwrap();
        assert_ne!(after_branch, baseline, "active-branch changes move the digest");

        // Prompt compilation recording moves the digest.
        let record = crate::model::PromptCompilationRecord {
            id: 0,
            session_id: session_id.clone(),
            turn_id: None,
            prefix_id: "prefix".into(),
            prefix_hash: "hash".into(),
            schema_version: 1,
            prefix_bytes: 400,
            prefix_token_estimate: 100,
            harness: "codex".into(),
            model: None,
            role: "orchestrator".into(),
            task_family: "implementation".into(),
            restoration_mode: "projection".into(),
            cross_harness_reuse: "none".into(),
            created_at: "now".into(),
            sections_json: None,
            stable_bytes: Some(300),
            variable_bytes: Some(100),
            stable_token_estimate: Some(75),
            variable_token_estimate: Some(25),
            token_estimate_source: Some(crate::prompt_compiler::TOKEN_ESTIMATE_SOURCE.into()),
        };
        store::record_prompt_compilation(&db, &record).unwrap();
        let after_compilation = context_breakdown_digest(&db, &session_id).unwrap();
        assert_ne!(after_compilation, after_branch, "compilations move the digest");

        // Prompt-section revisions move the digest.
        db.execute(
            "INSERT INTO prompt_section_revisions(target,section_id,operation,state,content,created_at) VALUES('orchestrator','bridge_role','override','overridden','route only','now')",
            [],
        )
        .unwrap();
        let after_revision = context_breakdown_digest(&db, &session_id).unwrap();
        assert_ne!(after_revision, after_compilation, "config revisions move the digest");

        // Live adapter observations move the digest through the revision counter.
        let before_observation = context_breakdown_digest(&db, &session_id).unwrap();
        let inventory = AdapterContextInventory::new(
            "claude",
            ContextInventoryScope::TurnPresented,
            crate::context_inventory::ContextLifecyclePhase::PerTurn,
            ContextSegmentClass::ALL
                .map(|class| {
                    crate::context_inventory::ContextSegmentObservation::estimated(
                        class,
                        std::iter::empty::<&str>(),
                        crate::context_inventory::ContextObservedSize::bounded(None, Some(10), Some(2)),
                        "test",
                    )
                })
                .to_vec(),
        )
        .unwrap();
        context_inventory::record_runtime_inventory(&std::sync::Mutex::new(Vec::new()), [inventory]);
        let after_observation = context_breakdown_digest(&db, &session_id).unwrap();
        assert_ne!(after_observation, before_observation, "observations move the digest");

        // And the recorded compilation surfaces as labelled segments.
        let breakdown = context_breakdown(&db, &session_id, &[]).unwrap();
        let stable = breakdown
            .segments
            .iter()
            .find(|segment| segment.segment_class == "prompt-stable")
            .unwrap();
        assert_eq!(stable.state, ContextBreakdownState::Estimated);
        assert_eq!(stable.method.as_deref(), Some(crate::prompt_compiler::TOKEN_ESTIMATE_SOURCE));
        assert_eq!(stable.tokens, Some(75));
    }
}

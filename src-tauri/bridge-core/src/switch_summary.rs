//! The outgoing model's handoff summary, run *after* a model switch commits.
//!
//! A cross-harness switch used to block the invoke for up to twenty seconds
//! while the outgoing provider produced a checkpoint, and nearly every switch
//! with a hot provider timed out and recorded a spurious `compaction.failed`.
//! Here the switch commits immediately on Bridge's mechanical projection, and
//! the outgoing runtime is *detached* — kept alive with its reader thread, but
//! moved out of the session's adapter slot so the incoming model can take that
//! slot. A background waiter delivers the checkpoint prompt to the detached
//! runtime and lets it summarise under the controller's own 30-second budget.
//!
//! The detached runtime and the incoming model share one `sessions` row, so
//! the detached path touches none of that row's turn state: its request is a
//! `background` [`crate::compaction_controller::PendingCompaction`] that lives
//! entirely in the forest, its frames are handled here rather than by
//! [`crate::live_turn::handle_agent_value`], and it never writes
//! `sessions.status` or `sessions.active_turn_id`.

use crate::{
    adapters,
    compaction_controller::{self, CheckpointOutcome, CompactionController, CompactionReason},
    runtime::BridgeCore,
    sessions::SwitchSummaryRequest,
    store, worker_guard,
};
use rusqlite::params;
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

/// A model switch's outgoing runtime, detached and summarising in the
/// background. Owns the runtime so the waiter can stop it and the frame
/// handler can resend a repair prompt.
pub struct DetachedSummary {
    runtime: Box<dyn adapters::AdapterRuntime>,
    request: SwitchSummaryRequest,
    adapter_id: String,
    process_id: u32,
    provider_session_id: String,
    /// Whether an assistant reply has been seen for the turn in flight; reset
    /// on `turn.started`.
    saw_reply: bool,
}

/// Move a session's live runtime out of its adapter slot into the detached
/// summary map, so the switch can commit and start the incoming model on the
/// same session while the outgoing runtime keeps its reader thread. Returns
/// `false` (and detaches nothing) when the session has no live runtime.
pub fn detach(core: &Arc<BridgeCore>, session_id: &str, request: SwitchSummaryRequest) -> bool {
    // The reader thread keeps running: its gate stays open so it can drain the
    // summary turn. Only the runtime handle moves out of the adapter slot.
    let Some(runtime) = core.adapters.lock().unwrap().remove(session_id) else {
        return false;
    };
    let process_id = runtime.process_id();
    let provider_session_id = runtime.provider_session_id().to_owned();
    let adapter_id: String = core
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT harness FROM sessions WHERE id=?1",
            params![session_id],
            |row| row.get(0),
        )
        .unwrap_or_default();
    core.detached_summaries.lock().unwrap().insert(
        session_id.to_owned(),
        DetachedSummary {
            runtime,
            request,
            adapter_id,
            process_id,
            provider_session_id,
            saw_reply: false,
        },
    );
    true
}

/// Whether the reader launch identified by `process_id`/`provider_session_id`
/// is a detached summary runtime, so its frames route here instead of to the
/// live conversation handler.
pub fn is_detached_launch(
    core: &BridgeCore,
    session_id: &str,
    process_id: u32,
    provider_session_id: &str,
) -> bool {
    core.detached_summaries
        .lock()
        .unwrap()
        .get(session_id)
        .is_some_and(|detached| {
            detached.process_id == process_id
                && detached.provider_session_id == provider_session_id
        })
}

/// Deliver the checkpoint prompt to the detached runtime (post-commit, so its
/// `checkpoint.turn_started` audit event lands after `session.model_changed`),
/// then wait for a terminal outcome under the controller's budget and clean
/// up. Runs on its own thread; failures are recorded, never propagated.
pub fn deliver_and_wait(core: Arc<BridgeCore>, session_id: String) {
    let request = {
        let map = core.detached_summaries.lock().unwrap();
        match map.get(&session_id) {
            Some(detached) => detached.request.clone(),
            None => return,
        }
    };
    // Send through the detached runtime without touching the session row's
    // status: the incoming model owns that row now.
    let delivery = {
        let map = core.detached_summaries.lock().unwrap();
        map.get(&session_id)
            .map(|detached| detached.runtime.send_turn(&request.prompt))
    };
    match delivery {
        Some(Ok(())) => {
            let _ = store::event(
                &core.db.lock().unwrap(),
                "compaction",
                "checkpoint.turn_started",
                &session_id,
                "Background model-switch summary turn started",
            );
        }
        Some(Err(error)) => {
            let _ = core.cancel_switch_summary(
                &session_id,
                &format!("model-switch summary could not be delivered: {error}"),
                0,
            );
            finish(&core, &session_id, adapters::ShutdownReason::Failed);
            return;
        }
        None => return,
    }

    let deadline =
        Instant::now() + Duration::from_secs(compaction_controller::CHECKPOINT_TIMEOUT_SECONDS.max(0) as u64);
    loop {
        match core.switch_summary_outcome(&request) {
            Ok(crate::sessions::SwitchSummaryOutcome::Summarised)
            | Ok(crate::sessions::SwitchSummaryOutcome::Failed) => break,
            Ok(crate::sessions::SwitchSummaryOutcome::Pending) => {}
            Err(_) => {
                let _ = core.cancel_switch_summary(
                    &session_id,
                    "model-switch summary wait failed after the switch; stored history carried",
                    1,
                );
                break;
            }
        }
        if Instant::now() >= deadline {
            let _ = core.cancel_switch_summary(
                &session_id,
                "model-switch summary timed out after the switch; stored history carried",
                1,
            );
            break;
        }
        std::thread::sleep(Duration::from_millis(300));
    }

    // A genuine failure still leaves the incoming model something: reconstruct
    // a checkpoint from the immutable events and Git facts, but only while the
    // new model has not spoken — a reconstructed boundary drawn over its turns
    // would summarise them out of its own projection.
    let failed = matches!(
        core.switch_summary_outcome(&request),
        Ok(crate::sessions::SwitchSummaryOutcome::Failed)
    );
    if failed {
        let new_model_spoke = compaction_controller::conversation_appended_since_request(
            &core.db.lock().unwrap(),
            &session_id,
        )
        .unwrap_or(true);
        if !new_model_spoke {
            reconstruct_before_downgrade(&core, &session_id);
        }
    }
    finish(&core, &session_id, adapters::ShutdownReason::Completed);
}

/// Handle one raw frame from a detached summary runtime. Only the checkpoint
/// pipeline is driven; the session row's status and turn state are never
/// touched.
pub fn handle_detached_frame(core: &Arc<BridgeCore>, session_id: &str, value: &serde_json::Value) {
    let adapter_id = {
        let map = core.detached_summaries.lock().unwrap();
        match map.get(session_id) {
            Some(detached) => detached.adapter_id.clone(),
            None => return,
        }
    };
    let events = core.adapter_registry.normalize(&adapter_id, value);
    for event in events {
        match event.kind.as_str() {
            "turn.started" => {
                if let Some(detached) = core.detached_summaries.lock().unwrap().get_mut(session_id) {
                    detached.saw_reply = false;
                }
            }
            "message.completed" if event.role.as_deref() == Some("assistant") => {
                set_saw_reply(core, session_id);
                let output = event.text.as_deref().unwrap_or_default();
                let outcome = {
                    let db = core.db.lock().unwrap();
                    CompactionController::handle_output(&db, session_id, output)
                };
                match outcome {
                    Ok(CheckpointOutcome::Repair { prompt }) => resend(core, session_id, &prompt),
                    Ok(_) | Err(_) => {}
                }
            }
            "turn.completed" => {
                let saw_reply = core
                    .detached_summaries
                    .lock()
                    .unwrap()
                    .get(session_id)
                    .map(|detached| detached.saw_reply)
                    .unwrap_or(true);
                if saw_reply {
                    continue;
                }
                // A turn that produced no assistant reply at all. The waiter
                // reconstructs from the immutable events after this failure, so
                // there is no need to hand-roll a repair turn here.
                let pending = {
                    let db = core.db.lock().unwrap();
                    CompactionController::pending(&db, session_id).ok().flatten()
                };
                if let Some(pending) = pending {
                    let _ = CompactionController::record_failure(
                        &core.db.lock().unwrap(),
                        session_id,
                        "checkpoint turn completed without an assistant response",
                        pending.attempt,
                    );
                }
            }
            _ => {}
        }
    }
}

/// The detached reader has exited (its provider process died). Record the
/// pending request's failure and forget the detached entry.
pub fn on_detached_reader_exit(core: &Arc<BridgeCore>, session_id: &str) {
    let pending = {
        let db = core.db.lock().unwrap();
        CompactionController::pending(&db, session_id).ok().flatten()
    };
    if let Some(pending) = pending {
        if pending.background {
            let _ = CompactionController::record_failure(
                &core.db.lock().unwrap(),
                session_id,
                "checkpoint turn ended because the adapter exited",
                pending.attempt,
            );
        }
    }
    core.detached_summaries.lock().unwrap().remove(session_id);
}

fn set_saw_reply(core: &Arc<BridgeCore>, session_id: &str) {
    if let Some(detached) = core.detached_summaries.lock().unwrap().get_mut(session_id) {
        detached.saw_reply = true;
    }
}

fn resend(core: &Arc<BridgeCore>, session_id: &str, prompt: &str) {
    let map = core.detached_summaries.lock().unwrap();
    if let Some(detached) = map.get(session_id) {
        let _ = detached.runtime.send_turn(prompt);
    }
}

fn reconstruct_before_downgrade(core: &Arc<BridgeCore>, session_id: &str) {
    let workspace_path: Option<String> = core
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT w.path FROM sessions s LEFT JOIN workspaces w ON w.id=s.workspace_id WHERE s.id=?1",
            params![session_id],
            |row| row.get(0),
        )
        .ok()
        .flatten();
    let git_status = workspace_path
        .as_deref()
        .map(|path| worker_guard::tracked_status(Path::new(path)).unwrap_or_default())
        .unwrap_or_default();
    let _ = CompactionController::reconstruct_from_normalized_events_and_git_with_reason(
        &core.db.lock().unwrap(),
        session_id,
        &git_status,
        CompactionReason::BeforeDowngrade,
    );
}

fn finish(core: &Arc<BridgeCore>, session_id: &str, reason: adapters::ShutdownReason) {
    // Stopping the runtime closes its stdout; the detached reader hits EOF and
    // exits, recognising itself as detached and skipping the live-session
    // teardown that would otherwise stop the incoming model.
    if let Some(mut detached) = core.detached_summaries.lock().unwrap().remove(session_id) {
        detached.runtime.stop(reason);
    }
}

/// Stop and forget a session's detached summary without recording a terminal
/// outcome — the cleanup path when the switch itself failed to commit. The
/// caller cancels the pending request.
pub fn abort(core: &Arc<BridgeCore>, session_id: &str) {
    finish(core, session_id, adapters::ShutdownReason::Replaced);
}

/// Whether a session currently has a detached summary in flight.
pub fn is_detached(core: &BridgeCore, session_id: &str) -> bool {
    core.detached_summaries
        .lock()
        .unwrap()
        .contains_key(session_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BridgeError;
    use crate::{
        adapters::{AdapterRegistry, AdapterRuntime, HarnessAdapter, ShutdownReason},
        agent::NormalizedEvent,
        compaction_controller::{CompactionController, CompactionReason},
        model::AdapterDescriptor,
        runtime::BridgeCore,
        session_forest::{EntryKind, SessionForest},
        store,
    };
    use serde_json::{json, Value};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex as StdMutex,
    };

    /// An adapter whose `normalize` turns compact test markers into the frames
    /// a checkpoint turn produces: `{"assistant":"<json>"}`, `{"turn":"started"}`,
    /// `{"turn":"completed"}`.
    struct SummaryTestAdapter;
    impl HarnessAdapter for SummaryTestAdapter {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn descriptor(&self) -> AdapterDescriptor {
            AdapterDescriptor {
                sandbox_modes: crate::model::SandboxMode::ALL.to_vec(),
                id: "codex".into(),
                label: "Codex".into(),
                available: true,
                auth_state: crate::model::AuthState::Unknown,
                version: None,
                capabilities: Vec::new(),
                unavailable_reason: None,
                models: Vec::new(),
                default_model: None,
                model_catalog: crate::model::ModelCatalogDiagnostics::curated(),
            }
        }
        fn start(
            &self,
            _: crate::adapters::StartRequest<'_>,
        ) -> Result<crate::adapters::StartedAdapter, BridgeError> {
            Err(BridgeError::Adapter("test adapter cannot start".into()))
        }
        fn resume(
            &self,
            _: crate::adapters::ResumeRequest<'_>,
        ) -> Result<crate::adapters::StartedAdapter, BridgeError> {
            Err(BridgeError::Adapter("test adapter cannot resume".into()))
        }
        fn supports_native_resume(&self) -> bool {
            false
        }
        fn normalize(&self, value: &Value) -> Vec<NormalizedEvent> {
            if let Some(text) = value.get("assistant").and_then(Value::as_str) {
                return vec![NormalizedEvent {
                    kind: "message.completed".into(),
                    item_id: Some("m".into()),
                    role: Some("assistant".into()),
                    status: Some("completed".into()),
                    title: None,
                    text: Some(text.to_owned()),
                    data: json!({}),
                }];
            }
            if let Some(phase) = value.get("turn").and_then(Value::as_str) {
                return vec![NormalizedEvent {
                    kind: format!("turn.{phase}"),
                    item_id: None,
                    role: None,
                    status: Some("ok".into()),
                    title: None,
                    text: None,
                    data: json!({}),
                }];
            }
            Vec::new()
        }
    }

    #[derive(Default)]
    struct SummaryTestRuntime {
        sent: Arc<StdMutex<Vec<String>>>,
        stops: Arc<AtomicUsize>,
    }
    impl AdapterRuntime for SummaryTestRuntime {
        fn process_id(&self) -> u32 {
            4242
        }
        fn provider_session_id(&self) -> &str {
            "outgoing-thread"
        }
        fn current_turn(&self) -> Arc<std::sync::Mutex<Option<String>>> {
            Arc::new(std::sync::Mutex::new(None))
        }
        fn send_turn(&self, text: &str) -> Result<(), BridgeError> {
            self.sent.lock().unwrap().push(text.to_owned());
            Ok(())
        }
        fn interrupt(&self) -> Result<(), BridgeError> {
            Ok(())
        }
        fn respond(&self, _: Value, _: &str) -> Result<(), BridgeError> {
            Ok(())
        }
        fn read_usage(&self) -> Result<(), BridgeError> {
            Ok(())
        }
        fn stop(&mut self, _: ShutdownReason) {
            self.stops.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn core_with_session() -> Arc<BridgeCore> {
        let scratch = Box::leak(Box::new(tempfile::tempdir().unwrap()));
        let mut core = BridgeCore::for_tests(scratch.path());
        let mut registry = AdapterRegistry::empty();
        registry.register(Box::new(SummaryTestAdapter)).unwrap();
        core.adapter_registry = Arc::new(registry);
        let core = Arc::new(core);
        {
            let db = core.db.lock().unwrap();
            db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','D','/tmp/ss','now')", []).unwrap();
            db.execute("INSERT INTO workspaces(id,project_id,title,path,status,created_at) VALUES('w','p','T','/tmp/ss','idle','now')", []).unwrap();
            db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind) VALUES('s','w','codex','Chat','idle','estimated','direct')", []).unwrap();
        }
        core
    }

    fn insert_runtime(core: &Arc<BridgeCore>) -> (Arc<StdMutex<Vec<String>>>, Arc<AtomicUsize>) {
        let runtime = SummaryTestRuntime::default();
        let sent = runtime.sent.clone();
        let stops = runtime.stops.clone();
        core.adapters.lock().unwrap().insert("s".into(), Box::new(runtime));
        (sent, stops)
    }

    fn background_request(core: &Arc<BridgeCore>) -> SwitchSummaryRequest {
        let db = core.db.lock().unwrap();
        let prompt = CompactionController::begin_background(&db, "s", CompactionReason::BeforeDowngrade, 100)
            .unwrap()
            .unwrap();
        let after: i64 = db
            .query_row(
                "SELECT COALESCE(MAX(sequence),0) FROM session_entries WHERE session_id='s' AND kind='compaction.requested'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        SwitchSummaryRequest { session_id: "s".into(), prompt, after_sequence: after }
    }

    fn valid_checkpoint(core: &Arc<BridgeCore>) -> String {
        let db = core.db.lock().unwrap();
        let pending = CompactionController::pending(&db, "s").unwrap().unwrap();
        json!({
            "schemaVersion":1,"summary":"what the old model knew","decisions":[],"filesTouched":[],
            "sourceAgent":"s","firstRetainedEntryId":pending.first_retained_entry_id,
            "tokensBefore":pending.tokens_before,"reason":pending.reason.as_str()
        })
        .to_string()
    }

    #[test]
    fn detach_moves_the_runtime_out_of_the_adapter_map_without_killing_its_reader() {
        let core = core_with_session();
        let (_sent, stops) = insert_runtime(&core);
        let request = background_request(&core);
        assert!(detach(&core, "s", request));
        assert!(!core.adapters.lock().unwrap().contains_key("s"), "moved out of the adapter slot");
        assert!(is_detached(&core, "s"));
        assert!(is_detached_launch(&core, "s", 4242, "outgoing-thread"));
        assert!(!is_detached_launch(&core, "s", 1, "some-other-thread"));
        assert_eq!(stops.load(Ordering::SeqCst), 0, "the runtime is kept alive, not stopped");
    }

    #[test]
    fn detach_reports_false_when_no_runtime_is_live() {
        let core = core_with_session();
        let request = background_request(&core);
        assert!(!detach(&core, "s", request));
        assert!(!is_detached(&core, "s"));
    }

    #[test]
    fn a_valid_reply_through_the_detached_handler_completes_the_request() {
        let core = core_with_session();
        let (_sent, _stops) = insert_runtime(&core);
        let request = background_request(&core);
        detach(&core, "s", request);
        let checkpoint = valid_checkpoint(&core);
        handle_detached_frame(&core, "s", &json!({"assistant": checkpoint}));
        let db = core.db.lock().unwrap();
        assert!(CompactionController::pending(&db, "s").unwrap().is_none(), "the request settled");
        let entries = store::session_entries(&db, "s").unwrap();
        assert!(entries.iter().any(|entry| entry.kind == "compaction"));
    }

    #[test]
    fn a_turn_without_a_reply_records_a_failure() {
        let core = core_with_session();
        let (_sent, _stops) = insert_runtime(&core);
        let request = background_request(&core);
        detach(&core, "s", request);
        handle_detached_frame(&core, "s", &json!({"turn": "started"}));
        handle_detached_frame(&core, "s", &json!({"turn": "completed"}));
        let db = core.db.lock().unwrap();
        assert!(CompactionController::pending(&db, "s").unwrap().is_none());
        let entries = store::session_entries(&db, "s").unwrap();
        assert!(entries.iter().any(|entry| entry.kind == "compaction.failed"));
    }

    #[test]
    fn the_detached_handler_never_touches_session_status_or_turn_state() {
        let core = core_with_session();
        let (_sent, _stops) = insert_runtime(&core);
        // The incoming model is mid-turn on the shared row.
        core.db.lock().unwrap().execute(
            "UPDATE sessions SET status='working', active_turn_id='new-model-turn' WHERE id='s'",
            [],
        ).unwrap();
        let request = background_request(&core);
        detach(&core, "s", request);
        handle_detached_frame(&core, "s", &json!({"turn": "started"}));
        let checkpoint = valid_checkpoint(&core);
        handle_detached_frame(&core, "s", &json!({"assistant": checkpoint}));
        handle_detached_frame(&core, "s", &json!({"turn": "completed"}));
        let (status, turn): (String, Option<String>) = core.db.lock().unwrap().query_row(
            "SELECT status, active_turn_id FROM sessions WHERE id='s'", [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(status, "working", "the incoming model's status is untouched");
        assert_eq!(turn.as_deref(), Some("new-model-turn"), "the incoming model's turn is untouched");
    }

    #[test]
    fn an_invalid_reply_repairs_then_the_repair_reply_settles() {
        let core = core_with_session();
        let (sent, _stops) = insert_runtime(&core);
        // Give the session a decision to carry, so the checkpoint must include it.
        SessionForest::new(&core.db.lock().unwrap())
            .append("s", EntryKind::WorkerResult, json!({"status":"completed","summary":"x","decisions":["Keep it"],"filesChanged":[]}))
            .unwrap();
        let request = background_request(&core);
        detach(&core, "s", request);
        // First reply is not valid JSON: the handler resends a repair prompt.
        handle_detached_frame(&core, "s", &json!({"assistant": "sorry, no"}));
        assert_eq!(sent.lock().unwrap().len(), 1, "a repair prompt was resent");
        assert!(sent.lock().unwrap()[0].contains("not valid against that schema"), "the resend is a repair prompt");
        // The repair reply carries the decision and settles.
        let db_pending = CompactionController::pending(&core.db.lock().unwrap(), "s").unwrap().unwrap();
        let repaired = json!({
            "schemaVersion":1,"summary":"done","decisions":["Keep it"],"filesTouched":[],
            "sourceAgent":"s","firstRetainedEntryId":db_pending.first_retained_entry_id,
            "tokensBefore":db_pending.tokens_before,"reason":db_pending.reason.as_str()
        }).to_string();
        handle_detached_frame(&core, "s", &json!({"assistant": repaired}));
        assert!(CompactionController::pending(&core.db.lock().unwrap(), "s").unwrap().is_none());
    }
}

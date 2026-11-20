//! Bound retained idle provider processes across all harnesses. Active turns,
//! approvals, workers, pending results/input and lifecycle operations are never victims.
//! History and provider session IDs remain on disk for the normal resume path.
use crate::{
    adapters::ShutdownReason, events::CoreEvent, runtime::BridgeCore,
    session_supervisor::SessionSupervisor,
};
use rusqlite::OptionalExtension;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

const MAX_IDLE_RUNTIMES: usize = 2;
const IDLE_TTL: Duration = Duration::from_secs(120);
/// How long an open harness subagent may stay silent and still hold its chat's
/// process. A live Claude or Codex child streams far more often than this (a
/// single shell call is capped at ten minutes); one the provider never closed
/// stops pinning the process once it has been quiet this long.
const SUBAGENT_QUIET_WINDOW_MINUTES: i64 = 30;

/// The statuses in which the idle release can stop a process.
const RELEASE_STATUSES: &[&str] = &["ready", "idle", "stopped"];

/// What holds one session, from one read. Each field is one clause of the
/// idle check, so a caller that refuses a change can say why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Activity {
    /// No parent and the kind `direct` or `orchestrator`.
    pub(crate) chat: bool,
    pub(crate) status: String,
    /// The status is in the list of the caller.
    pub(crate) status_fits: bool,
    pub(crate) active_turn: bool,
    /// A worker result or a `worker.result` outbox row that is not
    /// delivered yet.
    pub(crate) worker_result: bool,
    pub(crate) queued_input: bool,
    /// A harness subagent that waits for a person, or one that is open and
    /// spoke inside the quiet window.
    pub(crate) subagent: bool,
    /// A background task of the chat or of one of its harness subagents.
    pub(crate) background_task: bool,
}

impl Activity {
    pub(crate) fn idle(&self) -> bool {
        self.chat
            && self.status_fits
            && !self.active_turn
            && !self.worker_result
            && !self.queued_input
            && !self.subagent
            && !self.background_task
    }
}

/// The process is the only place a harness subagent or a background shell
/// runs. A chat that is idle in Bridge's eyes, because its own turn ended,
/// can still own both, and releasing it kills them mid-run. Bridge workers
/// were always protected; harness children and background tasks now are too.
/// `None` when the session does not exist.
pub(crate) fn activity(
    db: &rusqlite::Connection,
    id: &str,
    statuses: &[&str],
) -> Result<Option<Activity>, rusqlite::Error> {
    let statuses = serde_json::Value::from(statuses).to_string();
    db.query_row(
        "SELECT COALESCE(parent_session_id IS NULL AND kind IN ('direct','orchestrator'), 0),
         COALESCE(status, ''),
         status IS NOT NULL AND status IN (SELECT value FROM json_each(?3)),
         active_turn_id IS NOT NULL,
         EXISTS(SELECT 1 FROM worker_runtime WHERE parent_session_id=sessions.id AND result_status='pending')
         OR EXISTS(SELECT 1 FROM durable_outbox WHERE destination='parent' AND event_type='worker.result' AND status='pending' AND json_extract(payload,'$.report.parent_session_id')=sessions.id),
         EXISTS(SELECT 1 FROM queued_session_input WHERE session_id=sessions.id AND state IN ('queued','claiming')),
         EXISTS(SELECT 1 FROM sessions child
             WHERE child.parent_session_id=sessions.id AND child.kind='harness_subagent'
               -- An approval card is a person's to answer, however long it takes.
               AND (child.status='waiting'
                    OR (child.status IN ('working','starting','ready')
                        AND COALESCE((SELECT e.created_at FROM session_entries e WHERE e.session_id=child.id ORDER BY e.sequence DESC LIMIT 1), child.started_at, '')
                            >= strftime('%Y-%m-%dT%H:%M:%S','now',?2)))),
         EXISTS(SELECT 1 FROM session_background_tasks task
             WHERE task.in_progress=1
               AND (task.session_id=sessions.id OR task.session_id IN
                    (SELECT child.id FROM sessions child WHERE child.parent_session_id=sessions.id AND child.kind='harness_subagent')))
         FROM sessions WHERE id=?1",
        rusqlite::params![id, format!("-{SUBAGENT_QUIET_WINDOW_MINUTES} minutes"), statuses],
        |row| {
            Ok(Activity {
                chat: row.get(0)?,
                status: row.get(1)?,
                status_fits: row.get(2)?,
                active_turn: row.get(3)?,
                worker_result: row.get(4)?,
                queued_input: row.get(5)?,
                subagent: row.get(6)?,
                background_task: row.get(7)?,
            })
        },
    )
    .optional()
}

/// The shared idle check: a top-level chat in one of `statuses` that nothing
/// holds. The idle release and the save of a tool choice both use it.
pub(crate) fn idle_for_change(db: &rusqlite::Connection, id: &str, statuses: &[&str]) -> bool {
    activity(db, id, statuses).ok().flatten().is_some_and(|activity| activity.idle())
}

fn can_release(db: &rusqlite::Connection, id: &str) -> bool {
    idle_for_change(db, id, RELEASE_STATUSES)
}

#[derive(Default)]
pub(crate) struct IdleRuntimes {
    seen: HashMap<String, (u32, Instant)>,
}
impl IdleRuntimes {
    fn candidates(&mut self, eligible: &[(String, u32)], now: Instant) -> Vec<String> {
        let ids: HashSet<_> = eligible.iter().map(|(id, _)| id.as_str()).collect();
        self.seen.retain(|id, _| ids.contains(id.as_str()));
        for (id, pid) in eligible {
            let entry = self.seen.entry(id.clone()).or_insert((*pid, now));
            if entry.0 != *pid {
                *entry = (*pid, now);
            }
        }
        let mut ordered: Vec<_> = self
            .seen
            .iter()
            .map(|(id, (_, since))| (id.clone(), *since))
            .collect();
        ordered.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        let excess = ordered.len().saturating_sub(MAX_IDLE_RUNTIMES);
        ordered
            .into_iter()
            .enumerate()
            .filter(|(index, (_, since))| *index < excess || now.duration_since(*since) >= IDLE_TTL)
            .map(|(_, (id, _))| id)
            .collect()
    }

    pub(crate) fn maintain(&mut self, core: &Arc<BridgeCore>) {
        // Never queue resource housekeeping ahead of user input or DB work.
        let Ok(_input_lease) = core.input_activity.try_write() else {
            return;
        };
        let eligible = {
            let Ok(db) = core.db.try_lock() else {
                return;
            };
            let Ok(adapters) = core.adapters.try_lock() else {
                return;
            };
            adapters.iter().filter_map(|(id, runtime)| {
                let idle = can_release(&db, id);
                (idle && runtime.current_turn().lock().unwrap().is_none()).then(|| (id.clone(), runtime.process_id()))
            }).collect::<Vec<_>>()
        };
        for id in self.candidates(&eligible, Instant::now()) {
            let Ok(_lifecycle) = core.claim_session_lifecycle(&id, "idle resource release") else {
                continue;
            };
            let runtime = {
                let Ok(db) = core.db.try_lock() else {
                    continue;
                };
                let Ok(mut adapters) = core.adapters.try_lock() else {
                    continue;
                };
                let still_idle = can_release(&db, &id);
                if !still_idle {
                    continue;
                }
                let Some(runtime) = adapters.get(&id) else {
                    continue;
                };
                if runtime.current_turn().lock().unwrap().is_some() {
                    continue;
                }
                if SessionSupervisor::clear_adapter_process(&db, &id).is_err() {
                    continue;
                }
                core.deactivate_reader_launch(&id);
                adapters.remove(&id)
            };
            if let Some(mut runtime) = runtime {
                // No network abort or model-generated summary: this process is
                // already idle. Release it directly, outside database/map locks.
                drop(_input_lease);
                runtime.stop(ShutdownReason::Completed);
                // The reader was deactivated above, so its exit path will not
                // close what died with the process. A child left open here
                // (quiet past the window) would read as running forever.
                let closed = match core.try_db() {
                    Ok(db) => crate::harness_subagents::close_with_process(&db, &id),
                    Err(error) => {
                        eprintln!("[runtime-budget] cleanup for {id}: {error}");
                        Vec::new()
                    }
                };
                for event in closed {
                    core.events.publish(CoreEvent::Agent(event));
                }
                self.seen.remove(&id);
                core.events.publish(CoreEvent::StateChanged);
                // One release per tick bounds housekeeping and lets new input
                // proceed while process shutdown finishes.
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Runtime {
        core: std::sync::Weak<BridgeCore>,
        stopped: Arc<std::sync::atomic::AtomicUsize>,
    }
    impl crate::adapters::AdapterRuntime for Runtime {
        fn process_id(&self) -> u32 {
            42
        }
        fn provider_session_id(&self) -> &str {
            "saved"
        }
        fn current_turn(&self) -> Arc<std::sync::Mutex<Option<String>>> {
            Arc::new(std::sync::Mutex::new(None))
        }
        fn send_turn(&self, _: &str) -> Result<(), crate::BridgeError> {
            Ok(())
        }
        fn interrupt(&self) -> Result<(), crate::BridgeError> {
            panic!("idle release must not request generation or abort")
        }
        fn respond(&self, _: serde_json::Value, _: &str) -> Result<(), crate::BridgeError> {
            Ok(())
        }
        fn stop(&mut self, _: ShutdownReason) {
            let core = self.core.upgrade().unwrap();
            assert!(core.db.try_lock().is_ok());
            assert!(core.adapters.try_lock().is_ok());
            assert!(core.input_activity.try_read().is_ok());
            self.stopped
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    #[test]
    fn retirement_preserves_provider_resume_and_never_takes_active_or_submitting_chats() {
        let dir = tempfile::tempdir().unwrap();
        let core = Arc::new(BridgeCore::for_tests(dir.path()));
        let stopped = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        for (id, status) in [
            ("claude", "idle"),
            ("codex", "working"),
            ("opencode", "waiting"),
            ("cursor", "ready"),
        ] {
            core.db.lock().unwrap().execute("INSERT INTO sessions(id,harness,label,status,metric_source,kind,provider_session_id,adapter_pid) VALUES(?1,?1,?1,?2,'reported','direct','saved',42)", rusqlite::params![id,status]).unwrap();
            core.adapters.lock().unwrap().insert(
                id.into(),
                Box::new(Runtime {
                    core: Arc::downgrade(&core),
                    stopped: stopped.clone(),
                }),
            );
        }
        let mut pool = IdleRuntimes::default();
        for id in ["claude", "cursor"] {
            pool.seen.insert(id.into(), (42, Instant::now() - IDLE_TTL));
        }
        {
            let _sending = core.input_activity.read().unwrap();
            pool.maintain(&core);
            assert_eq!(core.adapters.lock().unwrap().len(), 4);
        }
        pool.maintain(&core);
        pool.maintain(&core);
        assert_eq!(stopped.load(std::sync::atomic::Ordering::SeqCst), 2);
        let adapters = core.adapters.lock().unwrap();
        assert!(adapters.contains_key("codex"));
        assert!(adapters.contains_key("opencode"));
        drop(adapters);
        let db = core.db.lock().unwrap();
        let saved: (String, Option<u32>) = db
            .query_row(
                "SELECT provider_session_id,adapter_pid FROM sessions WHERE id='claude'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(saved, ("saved".into(), None));
    }

    #[test]
    fn bounds_idle_processes_across_providers_and_expires_the_remainder() {
        let mut pool = IdleRuntimes::default();
        let now = Instant::now();
        let idle = ["claude", "codex", "opencode", "cursor"]
            .iter()
            .enumerate()
            .map(|(i, id)| (id.to_string(), i as u32))
            .collect::<Vec<_>>();
        assert_eq!(pool.candidates(&idle, now).len(), 2);
        assert_eq!(pool.candidates(&idle, now + IDLE_TTL).len(), 4);
        // Active/approval/startup sessions are absent from the eligible set.
        assert!(pool.candidates(&[], now + IDLE_TTL).is_empty());
        assert!(pool.seen.is_empty());
    }

    #[test]
    fn a_parent_waiting_for_a_worker_or_result_is_not_idle_capacity() {
        let dir = tempfile::tempdir().unwrap();
        let core = BridgeCore::for_tests(dir.path());
        let db = core.db.lock().unwrap();
        db.execute("INSERT INTO sessions(id,harness,label,status,metric_source,kind) VALUES('parent','codex','Parent','ready','reported','orchestrator')", []).unwrap();
        assert!(can_release(&db, "parent"));
        db.execute("INSERT INTO sessions(id,harness,label,status,metric_source,parent_session_id) VALUES('child','codex','Worker','working','reported','parent')", []).unwrap();
        db.execute("INSERT INTO worker_runtime(session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,result_status,updated_at) VALUES('child','parent','working','research','key','pending','now')", []).unwrap();
        assert!(!can_release(&db, "parent"));
        db.execute("UPDATE worker_runtime SET result_status='reported' WHERE session_id='child'", []).unwrap();
        db.execute("INSERT INTO durable_outbox(id,destination,event_type,payload,idempotency_key,status,next_attempt_at,created_at) VALUES('result','parent','worker.result','{\"report\":{\"parent_session_id\":\"parent\"}}','result','pending','now','now')", []).unwrap();
        assert!(!can_release(&db, "parent"));
        db.execute("UPDATE durable_outbox SET status='delivered' WHERE id='result'", []).unwrap();
        assert!(can_release(&db, "parent"));
    }
    #[test]
    fn a_chat_running_harness_subagents_or_background_shells_is_not_idle() {
        // Live 2026-11-17: background agents died about two minutes after the
        // chat's turn ended, every time, because this check let it go.
        let dir = tempfile::tempdir().unwrap();
        let core = BridgeCore::for_tests(dir.path());
        let db = core.db.lock().unwrap();
        db.execute("INSERT INTO sessions(id,harness,label,status,metric_source,kind,provider_session_id) VALUES('chat','claude','Chat','ready','reported','direct','root')", []).unwrap();
        assert!(can_release(&db, "chat"));
        let mut started = crate::agent::NormalizedEvent::new("subagent.started");
        started.data = serde_json::json!({"subtype":"task_started","taskId":"a98","subagent":{"sessionId":"agent-call","title":"Hunt"}});
        crate::harness_subagents::route(&db, "chat", &started, &serde_json::json!({})).unwrap();
        assert!(!can_release(&db, "chat"), "a working background agent holds the process");
        // Idle and quiet past the window: a child the provider never closed
        // must not pin the process forever.
        db.execute("UPDATE sessions SET status='ready' WHERE kind='harness_subagent'", []).unwrap();
        db.execute("UPDATE session_entries SET created_at='2020-01-01T00:00:00+00:00' WHERE session_id IN (SELECT id FROM sessions WHERE kind='harness_subagent')", []).unwrap();
        assert!(can_release(&db, "chat"));
        // Except a child waiting on a person's approval.
        db.execute("UPDATE sessions SET status='waiting' WHERE kind='harness_subagent'", []).unwrap();
        assert!(!can_release(&db, "chat"), "an open approval holds the process however quiet");
        db.execute("UPDATE sessions SET status='ready' WHERE kind='harness_subagent'", []).unwrap();
        let mut done = crate::agent::NormalizedEvent::new("subagent.completed");
        done.status = Some("completed".into());
        done.data = serde_json::json!({"subagent":{"sessionId":"agent-call"}});
        crate::harness_subagents::route(&db, "chat", &done, &serde_json::json!({})).unwrap();
        assert!(can_release(&db, "chat"));
        // A background shell, the chat's own or a child's, holds it too.
        for owner in ["chat", "child-shell"] {
            let mut shell = crate::agent::NormalizedEvent::new("tool.started");
            shell.item_id = Some(format!("background-task:{owner}"));
            shell.status = Some("inProgress".into());
            shell.data = serde_json::json!({"backgroundTask":true});
            if owner == "chat" {
                crate::store::session_event(&db, "chat", &shell, &serde_json::json!({})).unwrap();
            } else {
                shell.data["subagent"] = serde_json::json!({"sessionId":"agent-call"});
                crate::harness_subagents::route(&db, "chat", &shell, &serde_json::json!({})).unwrap();
            }
            assert!(!can_release(&db, "chat"), "{owner}");
            crate::background_activity::interrupt(&db, None).unwrap();
            assert!(can_release(&db, "chat"), "{owner}");
        }
    }
    /// The idle SQL of `can_release` before the shared check, without its
    /// one SQL comment.
    const OLD_CAN_RELEASE: &str = "SELECT parent_session_id IS NULL AND kind IN ('direct','orchestrator')
         AND status IN ('ready','idle','stopped') AND active_turn_id IS NULL
         AND NOT EXISTS(SELECT 1 FROM worker_runtime WHERE parent_session_id=sessions.id AND result_status='pending')
         AND NOT EXISTS(SELECT 1 FROM durable_outbox WHERE destination='parent' AND event_type='worker.result' AND status='pending' AND json_extract(payload,'$.report.parent_session_id')=sessions.id)
         AND NOT EXISTS(SELECT 1 FROM queued_session_input WHERE session_id=sessions.id AND state IN ('queued','claiming'))
         AND NOT EXISTS(SELECT 1 FROM sessions child
             WHERE child.parent_session_id=sessions.id AND child.kind='harness_subagent'
               AND (child.status='waiting'
                    OR (child.status IN ('working','starting','ready')
                        AND COALESCE((SELECT e.created_at FROM session_entries e WHERE e.session_id=child.id ORDER BY e.sequence DESC LIMIT 1), child.started_at, '')
                            >= strftime('%Y-%m-%dT%H:%M:%S','now',?2))))
         AND NOT EXISTS(SELECT 1 FROM session_background_tasks task
             WHERE task.in_progress=1
               AND (task.session_id=sessions.id OR task.session_id IN
                    (SELECT child.id FROM sessions child WHERE child.parent_session_id=sessions.id AND child.kind='harness_subagent')))
         FROM sessions WHERE id=?1";

    fn old_can_release(db: &rusqlite::Connection, id: &str, sql: &str) -> bool {
        db.query_row(sql, rusqlite::params![id, format!("-{SUBAGENT_QUIET_WINDOW_MINUTES} minutes")], |row| row.get::<_, bool>(0))
            .unwrap_or(false)
    }

    /// Each blocker, and whether a ready chat with only that blocker is idle.
    const BLOCKERS: &[(&str, bool)] = &[
        ("none", true),
        ("worker_pending", false),
        ("worker_reported", true),
        ("outbox_pending", false),
        ("outbox_delivered", true),
        ("queued", false),
        ("claiming", false),
        ("delivered", true),
        ("subagent_waiting", false),
        ("subagent_recent", false),
        ("subagent_quiet", true),
        ("subagent_spoke_recently", false),
        ("subagent_done", true),
        ("task_own", false),
        ("task_own_done", true),
        ("task_of_subagent", false),
    ];

    fn seed_blocker(db: &rusqlite::Connection, id: &str, blocker: &str) {
        let now = chrono::Utc::now().to_rfc3339();
        let quiet = "2020-01-01T00:00:00+00:00";
        let child = format!("{id}-child");
        let session = |kind: &str, status: &str, started: &str| {
            db.execute(
                "INSERT INTO sessions(id,harness,label,status,metric_source,kind,parent_session_id,started_at) VALUES(?1,'claude','Child',?2,'reported',?3,?4,?5)",
                rusqlite::params![child, status, kind, id, started],
            )
            .unwrap();
        };
        let task = |owner: &str, in_progress: bool| {
            db.execute("INSERT INTO session_background_tasks(session_id,item_id,sequence,in_progress) VALUES(?1,'task',1,?2)", rusqlite::params![owner, in_progress]).unwrap();
        };
        match blocker {
            "worker_pending" | "worker_reported" => {
                session("direct", "working", &now);
                let result = if blocker == "worker_pending" { "pending" } else { "reported" };
                db.execute("INSERT INTO worker_runtime(session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,result_status,updated_at) VALUES(?1,?2,'working','research','key',?3,'now')", rusqlite::params![child, id, result]).unwrap();
            }
            "outbox_pending" | "outbox_delivered" => {
                let status = if blocker == "outbox_pending" { "pending" } else { "delivered" };
                let payload = serde_json::json!({"report": {"parent_session_id": id}}).to_string();
                db.execute("INSERT INTO durable_outbox(id,destination,event_type,payload,idempotency_key,status,next_attempt_at,created_at) VALUES(?1,'parent','worker.result',?2,?1,?3,'now','now')", rusqlite::params![format!("{id}-result"), payload, status]).unwrap();
            }
            "queued" | "claiming" | "delivered" => {
                db.execute("INSERT INTO queued_session_input(id,session_id,provider_text,display_text,state,created_at) VALUES(?1,?2,'next','next',?3,'now')", rusqlite::params![format!("{id}-input"), id, blocker]).unwrap();
            }
            "subagent_waiting" => session("harness_subagent", "waiting", quiet),
            "subagent_recent" => session("harness_subagent", "working", &now),
            "subagent_quiet" => session("harness_subagent", "working", quiet),
            "subagent_spoke_recently" => {
                session("harness_subagent", "ready", quiet);
                db.execute("INSERT INTO session_entries(id,session_id,sequence,kind,payload,created_at) VALUES(?1,?2,1,'assistant.message','{}',?3)", rusqlite::params![format!("{id}-entry"), child, now]).unwrap();
            }
            "subagent_done" => session("harness_subagent", "completed", &now),
            "task_own" => task(id, true),
            "task_own_done" => task(id, false),
            "task_of_subagent" => {
                session("harness_subagent", "completed", quiet);
                task(&child, true);
            }
            _ => {}
        };
    }

    #[test]
    fn the_shared_idle_check_releases_the_same_sessions_as_before() {
        let dir = tempfile::tempdir().unwrap();
        let core = BridgeCore::for_tests(dir.path());
        let db = core.db.lock().unwrap();
        db.execute("INSERT INTO sessions(id,harness,label,status,metric_source,kind) VALUES('root','claude','Root','ready','reported','direct')", []).unwrap();
        let statuses = ["ready", "idle", "stopped", "failed", "working", "waiting", "checkpointing", "starting"];
        let insert = |id: &str, status: &str, kind: &str, parent: Option<&str>, turn: Option<&str>| {
            db.execute(
                "INSERT INTO sessions(id,harness,label,status,metric_source,kind,parent_session_id,active_turn_id) VALUES(?1,'claude','Chat',?2,'reported',?3,?4,?5)",
                rusqlite::params![id, status, kind, parent, turn],
            )
            .unwrap()
        };
        let mut serial = 0;
        let mut next_id = || {
            serial += 1;
            format!("s{serial:04}")
        };
        // The shape of a session: status, kind, parent and active turn.
        for status in statuses {
            for kind in ["direct", "orchestrator", "harness_subagent", "workflow"] {
                for parent in [None, Some("root")] {
                    for turn in [None, Some("turn-1")] {
                        insert(&next_id(), status, kind, parent, turn);
                    }
                }
            }
        }
        // Each blocker on each status of a top-level chat.
        let mut ready_chats = Vec::new();
        for status in statuses {
            for kind in ["direct", "orchestrator"] {
                for (blocker, idle) in BLOCKERS {
                    let id = next_id();
                    insert(&id, status, kind, None, None);
                    seed_blocker(&db, &id, blocker);
                    if status == "ready" {
                        ready_chats.push((id, *blocker, *idle));
                    }
                }
            }
        }
        // The fixtures reach each clause: a blocker holds a ready chat, and
        // a blocker that is over leaves it idle.
        for (id, blocker, idle) in &ready_chats {
            assert_eq!(old_can_release(&db, id, OLD_CAN_RELEASE), *idle, "{blocker}");
        }
        let with_failed = OLD_CAN_RELEASE.replace("('ready','idle','stopped')", "('ready','idle','stopped','failed')");
        assert_ne!(with_failed, OLD_CAN_RELEASE);
        let ids: Vec<String> = db
            .prepare("SELECT id FROM sessions ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let mut released = 0;
        for id in ids.iter().map(String::as_str).chain(["missing"]) {
            let before = old_can_release(&db, id, OLD_CAN_RELEASE);
            assert_eq!(can_release(&db, id), before, "{id}");
            assert_eq!(idle_for_change(&db, id, &["ready", "idle", "stopped", "failed"]), old_can_release(&db, id, &with_failed), "{id} with failed");
            released += usize::from(before);
        }
        assert!(released > 0 && released < ids.len(), "{released} of {}", ids.len());
    }

    #[test]
    fn a_new_process_or_resumed_activity_restarts_the_idle_deadline() {
        let mut pool = IdleRuntimes::default();
        let now = Instant::now();
        assert!(pool.candidates(&[("a".into(), 1)], now).is_empty());
        assert!(pool
            .candidates(&[("a".into(), 2)], now + IDLE_TTL)
            .is_empty());
        pool.candidates(&[], now + IDLE_TTL);
        assert!(pool
            .candidates(&[("a".into(), 2)], now + IDLE_TTL * 2)
            .is_empty());
    }
}

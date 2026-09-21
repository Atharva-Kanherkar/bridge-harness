//! Bound retained idle provider processes across all harnesses. Active turns,
//! approvals, workers, pending results/input and lifecycle operations are never victims.
//! History and provider session IDs remain on disk for the normal resume path.
use crate::{
    adapters::ShutdownReason, events::CoreEvent, runtime::BridgeCore,
    session_supervisor::SessionSupervisor,
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

const MAX_IDLE_RUNTIMES: usize = 2;
const IDLE_TTL: Duration = Duration::from_secs(120);

fn can_release(db: &rusqlite::Connection, id: &str) -> bool {
    db.query_row(
        "SELECT parent_session_id IS NULL AND kind IN ('direct','orchestrator')
         AND status IN ('ready','idle','stopped') AND active_turn_id IS NULL
         AND NOT EXISTS(SELECT 1 FROM worker_runtime WHERE parent_session_id=sessions.id AND result_status='pending')
         AND NOT EXISTS(SELECT 1 FROM durable_outbox WHERE destination='parent' AND event_type='worker.result' AND status='pending' AND json_extract(payload,'$.report.parent_session_id')=sessions.id)
         AND NOT EXISTS(SELECT 1 FROM queued_session_input WHERE session_id=sessions.id AND state IN ('queued','claiming'))
         FROM sessions WHERE id=?1", [id], |row| row.get::<_, bool>(0),
    ).unwrap_or(false)
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

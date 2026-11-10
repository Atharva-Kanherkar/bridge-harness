//! Durable views of children owned by a provider process. These sessions never
//! own a runtime, a worker policy or usage; all controls use the parent's adapter.
use crate::{agent::NormalizedEvent, model::AgentEvent, store, BridgeError};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

pub const KIND: &str = "harness_subagent";

pub fn is_child(db: &Connection, id: &str) -> Result<bool, BridgeError> {
    Ok(db
        .query_row(
            "SELECT kind=?2 FROM sessions WHERE id=?1",
            params![id, KIND],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(false))
}

pub fn require_process_session(db: &Connection, id: &str) -> Result<(), BridgeError> {
    if is_child(db, id)? {
        return Err(BridgeError::Invalid(
            "harness_subagent_processless: this subagent is controlled by its parent chat".into(),
        ));
    }
    Ok(())
}

pub fn adapter_owner(db: &Connection, id: &str) -> Result<String, BridgeError> {
    Ok(db.query_row(
        "SELECT CASE WHEN kind=?2 THEN parent_session_id ELSE id END FROM sessions WHERE id=?1",
        params![id, KIND],
        |r| r.get(0),
    )?)
}

/// Atomically create the child, advance its state and append its events and
/// parent pointers. The caller holds Bridge's correctness lock. Lifecycle
/// pointers deduplicate by child id, including replays and out-of-order starts.
pub fn route(
    db: &Connection,
    parent: &str,
    event: &NormalizedEvent,
    meta: &Value,
) -> Result<Vec<AgentEvent>, BridgeError> {
    let Some(tag) = event.data.get("subagent") else {
        return Ok(vec![]);
    };
    let Some(provider) = tag
        .get("sessionId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    else {
        return Err(BridgeError::Invalid(
            "subagent event has no provider identity".into(),
        ));
    };
    let tx = db.unchecked_transaction()?;
    let existing: Option<String> = tx.query_row("SELECT id FROM sessions WHERE parent_session_id=?1 AND provider_session_id=?2 AND kind=?3",params![parent,provider,KIND],|r|r.get(0)).optional()?;
    let created = existing.is_none();
    let child = existing.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let title = tag
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("Subagent");
    if created {
        tx.execute("INSERT OR IGNORE INTO sessions(id,workspace_id,harness,label,status,metric_source,model,kind,title,cwd,started_at,depth,parent_session_id,provider_session_id,account,trace_id)
        SELECT ?2,workspace_id,harness,?3,'working','estimated',model,?4,?3,cwd,strftime('%Y-%m-%dT%H:%M:%fZ','now'),COALESCE(depth,0)+1,id,?5,account,COALESCE(trace_id,id) FROM sessions WHERE id=?1",params![parent,child,title,KIND,provider])?;
    }
    // Replace fallback labels once the lifecycle's actual description arrives.
    if event.kind == "subagent.started" {
        tx.execute(
            "UPDATE sessions SET title=?2,label=?2 WHERE id=?1",
            params![child, title],
        )?;
    }
    let mut result = Vec::new();
    let mut pointer = event.clone();
    pointer.data["childSessionId"] = json!(child);
    pointer.text = None;
    pointer.kind = "subagent.started".into();
    pointer.status = Some("working".into());
    for kind in ["subagent.started", "subagent.completed"] {
        if kind == "subagent.completed" && !terminal(event) {
            continue;
        }
        let exists: bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM session_entries WHERE session_id=?1 AND kind=?2 AND json_extract(payload,'$.data.childSessionId')=?3)",params![parent,kind,child],|r|r.get(0))?;
        if !exists {
            pointer.kind = kind.into();
            if kind == "subagent.completed" {
                pointer.status = event.status.clone();
            }
            result.push(store::session_event_in_transaction(
                &tx, parent, &pointer, meta,
            )?);
        }
    }
    let status = if terminal(event) {
        match event.status.as_deref() {
            Some("failed" | "error") => "failed",
            Some("stopped" | "interrupted" | "cancelled") => "interrupted",
            _ => "completed",
        }
    } else if matches!(
        event.kind.as_str(),
        "permission.requested" | "question.requested"
    ) {
        "waiting"
    } else if event.kind == "subagent.status" && event.status.as_deref() == Some("idle") {
        "ready"
    } else {
        "working"
    };
    let turn = event
        .data
        .pointer("/turn/id")
        .or_else(|| event.data.get("turnId"))
        .and_then(Value::as_str);
    // A late tool/progress frame cannot revive a terminal child.
    if created
        || terminal(event)
        || matches!(
            event.kind.as_str(),
            "subagent.status" | "permission.requested" | "question.requested"
        )
    {
        tx.execute("UPDATE sessions SET status=CASE WHEN status='waiting' AND ?2='working' THEN status ELSE ?2 END,active_turn_id=CASE WHEN ?3 THEN NULL ELSE COALESCE(?4,active_turn_id) END,
        ended_at=CASE WHEN ?3 THEN COALESCE(ended_at,strftime('%Y-%m-%dT%H:%M:%fZ','now')) ELSE ended_at END
        WHERE id=?1 AND (status NOT IN ('completed','failed','interrupted') OR ?3)",params![child,status,terminal(event),turn])?;
    }
    // Child usage is already included by the provider's parent ledger/import.
    if event.kind != "usage.updated" {
        let mut child_event = event.clone();
        child_event.data["childSessionId"] = json!(child);
        result.push(store::session_event_in_transaction(
            &tx,
            &child,
            &child_event,
            meta,
        )?);
    }
    if terminal(event) {
        let pending = {
            let mut stmt=tx.prepare("SELECT e.sequence,e.kind FROM session_entries e WHERE e.session_id=?1 AND e.kind IN ('permission.requested','question.requested') AND NOT EXISTS (SELECT 1 FROM session_entries r WHERE r.session_id=e.session_id AND r.kind=replace(e.kind,'requested','resolved') AND json_extract(r.payload,'$.data.requestEventId')=e.sequence)")?;
            let rows = stmt
                .query_map([&child], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        for (sequence, kind) in pending {
            let mut settled = NormalizedEvent::new(&kind.replace("requested", "resolved"));
            settled.status = Some("cancelled".into());
            settled.data =
                json!({"requestEventId":sequence,"decision":"cancel","reason":"subagent_ended"});
            result.push(store::session_event_in_transaction(
                &tx, &child, &settled, meta,
            )?);
        }
    }
    tx.commit()?;
    Ok(result)
}

fn terminal(event: &NormalizedEvent) -> bool {
    event.kind == "subagent.completed"
        || event.data.get("turnCompleted").and_then(Value::as_bool) == Some(true)
}

/// Child identity is a provider tool id for Claude, while stopping needs its
/// task id. Never substitute the parent query/session id when that id is absent.
pub fn stop_target(
    db: &Connection,
    id: &str,
) -> Result<(String, String, Option<String>), BridgeError> {
    let (parent,provider,turn,harness):(String,String,Option<String>,String)=db.query_row("SELECT parent_session_id,provider_session_id,active_turn_id,harness FROM sessions WHERE id=?1 AND kind=?2",params![id,KIND],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
    let target = if harness == "claude" {
        db.query_row("SELECT json_extract(payload,'$.data.taskId') FROM session_entries WHERE session_id=?1 AND kind='subagent.started' AND json_extract(payload,'$.data.taskId') IS NOT NULL ORDER BY sequence LIMIT 1",params![id],|r|r.get(0)).optional()?.ok_or_else(||BridgeError::Invalid("Subagent task id has not arrived yet".into()))?
    } else {
        provider
    };
    Ok((parent, target, turn))
}

pub fn interrupt_orphans(db: &Connection) -> Result<usize, BridgeError> {
    interrupt_family(db, None)
}
pub fn interrupt_children(db: &Connection, parent: &str) -> Result<usize, BridgeError> {
    interrupt_family(db, Some(parent))
}
fn interrupt_family(db: &Connection, parent: Option<&str>) -> Result<usize, BridgeError> {
    let ids = {
        let mut stmt=db.prepare("SELECT parent_session_id,provider_session_id FROM sessions WHERE kind=?1 AND status IN ('working','waiting','starting','ready') AND (?2 IS NULL OR parent_session_id=?2)")?;
        let ids = stmt
            .query_map(params![KIND, parent], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        ids
    };
    for (parent, provider) in &ids {
        let mut event = NormalizedEvent::new("subagent.completed");
        event.status = Some("interrupted".into());
        event.data = json!({"subagent":{"sessionId":provider},"reason":"parent_process_exited"});
        route(db, parent, &event, &json!({"source":"recovery"}))?;
    }
    Ok(ids.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn db() -> Connection {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        db.execute("INSERT INTO sessions(id,harness,label,status,metric_source,kind,provider_session_id,active_turn_id,cwd) VALUES('parent','codex','Chat','working','estimated','direct','root','root-turn','/tmp')",[]).unwrap();
        db
    }
    fn event(kind: &str, id: &str) -> NormalizedEvent {
        let mut e = NormalizedEvent::new(kind);
        e.data = json!({"subagent":{"sessionId":id,"agent":"codex","title":"Research"}});
        e
    }
    #[test]
    fn interleaved_children_route_once_and_never_settle_parent() {
        let db = db();
        for child in ["one", "two", "three"] {
            for _ in 0..2 {
                route(&db, "parent", &event("subagent.started", child), &json!({})).unwrap();
            }
            let mut msg = event("message.completed", child);
            msg.role = Some("assistant".into());
            msg.text = Some(format!("Reply from {child}"));
            route(&db, "parent", &msg, &json!({})).unwrap();
            route(&db, "parent", &event("usage.updated", child), &json!({})).unwrap();
            let mut done = event("subagent.completed", child);
            done.status = Some("completed".into());
            for _ in 0..2 {
                route(&db, "parent", &done, &json!({})).unwrap();
            }
        }
        let count = |sql: &str| db.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
        assert_eq!(
            count("SELECT count(*) FROM sessions WHERE kind='harness_subagent'"),
            3
        );
        assert_eq!(count("SELECT count(*) FROM worker_runtime"), 0);
        assert_eq!(
            count("SELECT count(*) FROM session_entries WHERE session_id='parent'"),
            6
        );
        assert_eq!(count("SELECT count(*) FROM session_entries WHERE session_id='parent' AND context_visibility!='hidden'"),0);
        assert_eq!(count("SELECT count(*) FROM session_entries WHERE kind='assistant.message' AND session_id!='parent'"),3);
        assert_eq!(count("SELECT count(*) FROM usage_ledger"), 0);
        let parent: (String, String) = db
            .query_row(
                "SELECT status,active_turn_id FROM sessions WHERE id='parent'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(parent, ("working".into(), "root-turn".into()));
    }
    #[test]
    fn processless_guards_and_orphan_recovery() {
        let db = db();
        route(&db, "parent", &event("subagent.started", "one"), &json!({})).unwrap();
        let id: String = db
            .query_row(
                "SELECT id FROM sessions WHERE kind='harness_subagent'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(require_process_session(&db, &id)
            .unwrap_err()
            .to_string()
            .starts_with("harness_subagent_processless:"));
        assert_eq!(adapter_owner(&db, &id).unwrap(), "parent");
        assert_eq!(interrupt_orphans(&db).unwrap(), 1);
        assert_eq!(interrupt_orphans(&db).unwrap(), 0);
        let status: String = db
            .query_row("SELECT status FROM sessions WHERE id=?1", [&id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(status, "interrupted");
    }
    #[test]
    fn claude_background_replay_waits_for_notification() {
        let db = db();
        db.execute("UPDATE sessions SET harness='claude' WHERE id='parent'", [])
            .unwrap();
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../testing/fixtures/claude-subagents.json"
        ))
        .unwrap();
        let mut state = crate::agent::ClaudeStreamState::default();
        for (index, frame) in fixture["frames"].as_array().unwrap().iter().enumerate() {
            for event in crate::agent::normalize_claude_message_with_state(frame, &mut state) {
                assert!(!event
                    .text
                    .as_deref()
                    .unwrap_or("")
                    .contains("Async agent launched"));
                if event.data.get("subagent").is_some() {
                    route(&db, "parent", &event, &json!({})).unwrap();
                }
            }
            let status: String = db
                .query_row(
                    "SELECT status FROM sessions WHERE kind='harness_subagent'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(status, if index == 5 { "completed" } else { "working" });
        }
        let id: String = db
            .query_row(
                "SELECT id FROM sessions WHERE kind='harness_subagent'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stop_target(&db, &id).unwrap().1, "task-1");
    }
    #[test]
    fn ending_a_child_settles_its_pending_approval() {
        let db = db();
        let mut request = event("permission.requested", "one");
        request.data["requestId"] = json!(42);
        let events = route(&db, "parent", &request, &json!({})).unwrap();
        let child = events.last().unwrap().session_id.clone();
        let mut done = event("subagent.completed", "one");
        done.status = Some("interrupted".into());
        let events = route(&db, "parent", &done, &json!({})).unwrap();
        assert!(events.iter().any(|e| e.session_id == child
            && e.kind == "permission.resolved"
            && e.status.as_deref() == Some("cancelled")));
    }
}

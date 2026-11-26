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
    route_with_repository_state(db, parent, event, meta, None)
}

pub(crate) fn route_with_repository_state(
    db: &Connection,
    parent: &str,
    event: &NormalizedEvent,
    meta: &Value,
    repository_state: Option<&Value>,
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
    // A provider can name the parent's own thread (a Codex child messaging
    // `/root`). That is the chat, not a child: a row for it would never finish.
    let own: Option<String> = db
        .query_row(
            "SELECT provider_session_id FROM sessions WHERE id=?1",
            [parent],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    if own.as_deref() == Some(provider) {
        return Ok(vec![]);
    }
    let tx = db.unchecked_transaction()?;
    let mut existing: Option<String> = tx.query_row("SELECT id FROM sessions WHERE parent_session_id=?1 AND provider_session_id=?2 AND kind=?3",params![parent,provider,KIND],|r|r.get(0)).optional()?;
    // Claude names one agent by its task id for its whole life, while every
    // continuation (`SendMessage`, a resume after a restart) arrives under a
    // new tool id. The task id finds the child the first call created, so the
    // work and its completion stay on one row.
    let task = event.data.get("taskId").and_then(Value::as_str).filter(|task| !task.is_empty());
    if existing.is_none() && event.kind.starts_with("subagent.") {
        if let Some(task) = task {
            existing = tx.query_row(
                "SELECT c.id FROM sessions c WHERE c.parent_session_id=?1 AND c.kind=?3
                   AND EXISTS(SELECT 1 FROM session_entries e WHERE e.session_id=c.id AND e.kind='subagent.started'
                              AND json_extract(e.payload,'$.data.taskId')=?2)
                 ORDER BY c.started_at LIMIT 1",
                params![parent, task, KIND],
                |r| r.get(0),
            ).optional()?;
        }
    }
    // A lifecycle report for a task this chat never started (a background
    // shell, or an orphan reported by a restarted process) is not an agent.
    if existing.is_none() && tag.get("lookupOnly").and_then(Value::as_bool) == Some(true) {
        tx.commit()?;
        return Ok(vec![]);
    }
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
    let turn = event
        .data
        .pointer("/turn/id")
        .or_else(|| event.data.get("turnId"))
        .and_then(Value::as_str);
    let turn_started = event.data.get("turnStarted").and_then(Value::as_bool) == Some(true);
    let turn_completed = event.data.get("turnCompleted").and_then(Value::as_bool) == Some(true);
    if (turn_started || turn_completed) && turn.is_some() {
        let replayed:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM session_entries WHERE session_id=?1 AND kind='subagent.status' AND json_extract(payload,'$.data.turnCompleted')=1 AND COALESCE(json_extract(payload,'$.data.turn.id'),json_extract(payload,'$.data.turnId'))=?2)",params![child,turn],|r|r.get(0))?;
        let active: Option<String> = tx.query_row(
            "SELECT active_turn_id FROM sessions WHERE id=?1",
            [&child],
            |r| r.get(0),
        )?;
        if replayed || (turn_completed && active.as_deref().is_some_and(|id| Some(id) != turn)) {
            tx.commit()?;
            return Ok(vec![]);
        }
    }
    // A continued Claude agent runs again under the row it finished on.
    let reopened = !created
        && event.kind == "subagent.started"
        && event.data.get("subtype").and_then(Value::as_str) == Some("task_started");
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
    // An idle child doing something is working again; a late tool/progress
    // frame cannot revive a terminal one (the WHERE below keeps those).
    let revived = status == "working" && event.kind != "usage.updated" && !created && {
        let current: String = tx.query_row("SELECT status FROM sessions WHERE id=?1", [&child], |r| r.get(0))?;
        current == "ready"
    };
    if created
        || revived
        || reopened
        || turn_started
        || terminal(event)
        || matches!(
            event.kind.as_str(),
            "subagent.status" | "permission.requested" | "question.requested"
        )
    {
        tx.execute("UPDATE sessions SET status=CASE WHEN status='waiting' AND ?2='working' AND NOT ?5 THEN status ELSE ?2 END,active_turn_id=CASE WHEN ?3 THEN NULL ELSE COALESCE(?4,active_turn_id) END,
        ended_at=CASE WHEN ?5 THEN NULL WHEN ?3 THEN COALESCE(ended_at,strftime('%Y-%m-%dT%H:%M:%fZ','now')) ELSE ended_at END
        WHERE id=?1 AND (status NOT IN ('completed','failed','interrupted') OR ?3 OR ?5)",params![child,status,terminal(event),turn,turn_started || reopened])?;
    }
    // The caller bills child usage to the parent; the child keeps no copy.
    if event.kind != "usage.updated" {
        let mut child_event = event.clone();
        child_event.data["childSessionId"] = json!(child);
        let child_state = if repository_state.is_some()
            && store::repository_location_for_session(&tx, &child)?
                == store::repository_location_for_session(&tx, parent)?
        {
            repository_state
        } else {
            None
        };
        result.push(store::session_event_in_transaction_with_repository_state(
            &tx, &child, &child_event, meta, child_state,
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
    for event in &result {
        if event.sequence > 0 && crate::repo_stamp::changes_folder(&event.kind) {
            crate::repo_stamp::invalidate_session(db, &event.session_id);
        }
    }
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

/// The chat's turn is over, so nothing it started can still be running under
/// it. A child the provider never closed stays `working` forever otherwise,
/// and every live count (the sidebar, Mission Control) reports it. It goes
/// idle rather than terminal: a provider that really keeps a child running
/// past the turn (a Claude background agent) revives it with its next frame.
/// A child waiting on an approval keeps that state, since the card is real.
///
/// Claude is the exception. A foreground `Agent` call holds the turn open, so
/// a Claude child still open when the turn ends is a background agent, and
/// Claude reports its end with `task_notification`. Idling it would show a
/// running agent as finished; the process exit closes it if that never comes.
pub fn idle_children(db: &Connection, parent: &str) -> Result<usize, BridgeError> {
    Ok(db.execute(
        "UPDATE sessions SET status='ready',active_turn_id=NULL WHERE kind=?1 AND parent_session_id=?2 AND status IN ('working','starting') AND harness<>'claude'",
        params![KIND, parent],
    )?)
}

/// Everything a chat's process was running dies with it: its open harness
/// subagents and the background tasks of the chat and of those children. A
/// caller that ends a process after deactivating its reader (idle release, a
/// replacing launch) must close them itself, because that reader's exit path
/// no longer will. Returns the events to publish.
pub fn close_with_process(db: &Connection, parent: &str) -> Vec<AgentEvent> {
    let mut events = interrupt_family_events(db, Some(parent)).unwrap_or_default();
    let mut sessions = vec![parent.to_owned()];
    if let Ok(mut stmt) = db.prepare("SELECT id FROM sessions WHERE parent_session_id=?1 AND kind=?2") {
        if let Ok(rows) = stmt.query_map(params![parent, KIND], |r| r.get::<_, String>(0)) {
            sessions.extend(rows.filter_map(Result::ok));
        }
    }
    for session in sessions {
        if let Ok(closed) = crate::background_activity::interrupt(db, Some(&session)) {
            events.extend(closed);
        }
    }
    events
}

pub fn interrupt_orphans(db: &Connection) -> Result<usize, BridgeError> {
    interrupt_family(db, None)
}
pub fn interrupt_children(db: &Connection, parent: &str) -> Result<usize, BridgeError> {
    interrupt_family(db, Some(parent))
}
fn interrupt_family(db: &Connection, parent: Option<&str>) -> Result<usize, BridgeError> {
    let mut closed = 0;
    interrupt_family_with(db, parent, |_| closed += 1)?;
    Ok(closed)
}
fn interrupt_family_events(db: &Connection, parent: Option<&str>) -> Result<Vec<AgentEvent>, BridgeError> {
    let mut events = Vec::new();
    interrupt_family_with(db, parent, |stored| events.extend(stored))?;
    Ok(events)
}
fn interrupt_family_with(
    db: &Connection,
    parent: Option<&str>,
    mut on_child: impl FnMut(Vec<AgentEvent>),
) -> Result<(), BridgeError> {
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
        on_child(route(db, parent, &event, &json!({"source":"recovery"}))?);
    }
    Ok(())
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
    fn claude_db() -> Connection {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        db.execute("INSERT INTO sessions(id,harness,label,status,metric_source,kind,provider_session_id,cwd) VALUES('parent','claude','Chat','ready','estimated','direct','root','/tmp')",[]).unwrap();
        db
    }
    fn task(kind: &str, tool: &str, task: &str, subtype: &str) -> NormalizedEvent {
        let mut e = NormalizedEvent::new(kind);
        e.data = json!({"subtype":subtype,"taskId":task,"subagent":{"sessionId":tool,"agent":"Explore","title":"Map the registry"}});
        e
    }
    #[test]
    fn a_continued_claude_agent_stays_one_child_across_processes() {
        // Live 2026-11-17: the agent's work landed on the row of its first
        // call and its completion on a second row made for SendMessage.
        let db = claude_db();
        route(&db, "parent", &task("subagent.started", "agent-call", "a98", "task_started"), &json!({})).unwrap();
        let mut work = event("command.completed", "agent-call");
        work.status = Some("completed".into());
        route(&db, "parent", &work, &json!({})).unwrap();
        // A restarted process announces the continuation under a new tool id.
        route(&db, "parent", &task("subagent.started", "send-message", "a98", "task_started"), &json!({})).unwrap();
        let mut done = task("subagent.completed", "send-message", "a98", "task_notification");
        done.status = Some("completed".into());
        route(&db, "parent", &done, &json!({})).unwrap();
        let rows: Vec<(String, String)> = db.prepare("SELECT provider_session_id,status FROM sessions WHERE kind='harness_subagent'").unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
        assert_eq!(rows, [("agent-call".to_owned(), "completed".to_owned())]);
    }
    #[test]
    fn a_continuation_reopens_a_finished_claude_agent() {
        let db = claude_db();
        route(&db, "parent", &task("subagent.started", "agent-call", "a98", "task_started"), &json!({})).unwrap();
        let mut done = task("subagent.completed", "agent-call", "a98", "task_notification");
        done.status = Some("completed".into());
        route(&db, "parent", &done, &json!({})).unwrap();
        route(&db, "parent", &task("subagent.started", "agent-call", "a98", "task_started"), &json!({})).unwrap();
        let (status, ended): (String, Option<String>) = db.query_row("SELECT status,ended_at FROM sessions WHERE kind='harness_subagent'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((status.as_str(), ended), ("working", None));
        // An out-of-order start without Claude's task lifecycle does not.
        route(&db, "parent", &done, &json!({})).unwrap();
        route(&db, "parent", &event("subagent.started", "agent-call"), &json!({})).unwrap();
        let status: String = db.query_row("SELECT status FROM sessions WHERE kind='harness_subagent'", [], |r| r.get(0)).unwrap();
        assert_eq!(status, "completed");
    }
    #[test]
    fn a_report_about_an_unknown_task_never_creates_a_row() {
        // Live: 118 one-entry "Claude subagent" rows from shell and orphan
        // notifications after a restart.
        let db = claude_db();
        let mut orphan = task("subagent.completed", "b1pwyirf3", "b1pwyirf3", "task_notification");
        orphan.status = Some("stopped".into());
        orphan.data["subagent"]["lookupOnly"] = json!(true);
        assert!(route(&db, "parent", &orphan, &json!({})).unwrap().is_empty());
        let count: i64 = db.query_row("SELECT count(*) FROM sessions WHERE kind='harness_subagent'", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);
        // The same report for an agent this chat did start closes that row.
        route(&db, "parent", &task("subagent.started", "agent-call", "a98", "task_started"), &json!({})).unwrap();
        let mut killed = task("subagent.completed", "a98", "a98", "task_notification");
        killed.status = Some("stopped".into());
        killed.data["subagent"]["lookupOnly"] = json!(true);
        assert!(!route(&db, "parent", &killed, &json!({})).unwrap().is_empty());
        let rows: Vec<(String, String)> = db.prepare("SELECT provider_session_id,status FROM sessions WHERE kind='harness_subagent'").unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
        assert_eq!(rows, [("agent-call".to_owned(), "interrupted".to_owned())]);
    }
    #[test]
    fn a_finished_claude_turn_leaves_its_background_agents_running() {
        let db = claude_db();
        route(&db, "parent", &task("subagent.started", "agent-call", "a98", "task_started"), &json!({})).unwrap();
        assert_eq!(idle_children(&db, "parent").unwrap(), 0);
        let status: String = db.query_row("SELECT status FROM sessions WHERE kind='harness_subagent'", [], |r| r.get(0)).unwrap();
        assert_eq!(status, "working");
    }
    #[test]
    fn closing_with_the_process_ends_children_and_their_background_tasks() {
        let db = claude_db();
        route(&db, "parent", &task("subagent.started", "agent-call", "a98", "task_started"), &json!({})).unwrap();
        let mut shell = event("tool.started", "agent-call");
        shell.item_id = Some("background-task:bcav".into());
        shell.status = Some("inProgress".into());
        shell.data["backgroundTask"] = json!(true);
        route(&db, "parent", &shell, &json!({})).unwrap();
        let open = |db: &Connection| db.query_row("SELECT count(*) FROM session_background_tasks WHERE in_progress=1", [], |r| r.get::<_, i64>(0)).unwrap();
        assert_eq!(open(&db), 1);
        let events = close_with_process(&db, "parent");
        assert!(events.iter().any(|e| e.kind == "subagent.completed"));
        assert_eq!(open(&db), 0);
        let status: String = db.query_row("SELECT status FROM sessions WHERE kind='harness_subagent'", [], |r| r.get(0)).unwrap();
        assert_eq!(status, "interrupted");
        assert!(close_with_process(&db, "parent").is_empty());
    }
    #[test]
    fn the_parents_own_thread_never_becomes_its_child() {
        let db = db();
        for kind in ["subagent.started", "subagent.progress", "message.completed"] {
            assert!(route(&db, "parent", &event(kind, "root"), &json!({})).unwrap().is_empty());
        }
        let children: i64 = db
            .query_row("SELECT count(*) FROM sessions WHERE kind='harness_subagent'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(children, 0);
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
    fn a_finished_turn_idles_unclosed_children_until_they_act_again() {
        let db = db();
        let status = |provider: &str| -> String {
            db.query_row("SELECT status FROM sessions WHERE provider_session_id=?1 AND kind='harness_subagent'", [provider], |r| r.get(0)).unwrap()
        };
        for child in ["silent", "asking", "done"] {
            route(&db, "parent", &event("subagent.started", child), &json!({})).unwrap();
        }
        route(&db, "parent", &event("permission.requested", "asking"), &json!({})).unwrap();
        let mut done = event("subagent.completed", "done");
        done.status = Some("completed".into());
        route(&db, "parent", &done, &json!({})).unwrap();

        assert_eq!(idle_children(&db, "parent").unwrap(), 1);
        assert_eq!(status("silent"), "ready");
        assert_eq!(status("asking"), "waiting");
        assert_eq!(status("done"), "completed");

        // usage alone is not activity; a real frame is.
        route(&db, "parent", &event("usage.updated", "silent"), &json!({})).unwrap();
        assert_eq!(status("silent"), "ready");
        route(&db, "parent", &event("tool.started", "silent"), &json!({})).unwrap();
        assert_eq!(status("silent"), "working");
        route(&db, "parent", &event("tool.started", "done"), &json!({})).unwrap();
        assert_eq!(status("done"), "completed");
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
    #[test]
    fn new_child_turns_reopen_the_session_but_old_turn_replays_do_not() {
        let db = db();
        let frame = |kind: &str, turn: &str| {
            let mut e = event("subagent.status", "one");
            e.status = Some(
                if kind == "turnStarted" {
                    "working"
                } else {
                    "completed"
                }
                .into(),
            );
            e.data[kind] = json!(true);
            e.data["turn"] = json!({"id":turn});
            e
        };
        route(&db, "parent", &frame("turnStarted", "first"), &json!({})).unwrap();
        route(&db, "parent", &frame("turnCompleted", "first"), &json!({})).unwrap();
        assert!(
            route(&db, "parent", &frame("turnStarted", "first"), &json!({}))
                .unwrap()
                .is_empty()
        );
        route(&db, "parent", &frame("turnStarted", "second"), &json!({})).unwrap();
        assert!(
            route(&db, "parent", &frame("turnCompleted", "first"), &json!({}))
                .unwrap()
                .is_empty()
        );
        let (status, turn, ended): (String, String, Option<String>) = db
            .query_row(
                "SELECT status,active_turn_id,ended_at FROM sessions WHERE kind='harness_subagent'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (status, turn, ended),
            ("working".into(), "second".into(), None)
        );
        route(&db, "parent", &frame("turnCompleted", "second"), &json!({})).unwrap();
        let pointers: i64 = db
            .query_row(
                "SELECT count(*) FROM session_entries WHERE session_id='parent'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(pointers, 2);
    }
}

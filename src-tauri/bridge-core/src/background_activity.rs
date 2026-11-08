//! Clear claims of live background work when its owning runtime is gone.
use crate::{agent::NormalizedEvent, model::AgentEvent, store, BridgeError};
use rusqlite::{params, Connection};
use serde_json::Value;

/// Close the latest task snapshots without inferring an outcome. `None` is
/// startup recovery; a session id is a generation-checked reader exit.
pub(crate) fn interrupt(
    db: &Connection,
    session_id: Option<&str>,
) -> Result<Vec<AgentEvent>, BridgeError> {
    // Keep the grouped process set outside the entry lookup. SQLite otherwise
    // restarts its coroutine once per history row (quadratic on long sessions).
    let mut statement = db.prepare(
        "SELECT e.session_id,e.payload FROM (SELECT session_id,json_extract(CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,'$.itemId') AS item_id,MAX(sequence) AS sequence
               FROM session_entries WHERE kind IN ('tool.started','tool.completed','command.started','command.completed')
                 AND json_extract(CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,'$.data.backgroundTask')=1
                 AND (?1 IS NULL OR session_id=?1)
               GROUP BY session_id,json_extract(CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,'$.itemId')) latest CROSS JOIN session_entries e
           ON e.session_id=latest.session_id AND e.sequence=latest.sequence
         WHERE json_extract(CASE WHEN json_valid(e.payload) THEN e.payload ELSE '{}' END,'$.status')='inProgress'",
    )?;
    let rows: Vec<(String, String)> = statement
        .query_map(params![session_id], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<_, _>>()?;
    drop(statement);
    rows.into_iter()
        .map(|(session, payload)| {
            let payload: Value = serde_json::from_str(&payload).map_err(|error| {
                BridgeError::Invalid(format!("invalid background task snapshot: {error}"))
            })?;
            let mut event = NormalizedEvent::new("tool.completed");
            event.item_id = payload
                .get("itemId")
                .and_then(Value::as_str)
                .map(str::to_owned);
            event.title = payload
                .get("title")
                .and_then(Value::as_str)
                .map(str::to_owned);
            event.text = payload
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_owned);
            event.status = Some("unknown".into());
            event.data = payload
                .get("data")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            event.data["background"] = Value::Bool(false);
            event.data["progress"] = Value::String("Runtime ended. Outcome unavailable.".into());
            store::session_event(
                db,
                &session,
                &event,
                &serde_json::json!({"source":"runtime_recovery"}),
            )
        })
        .collect()
}

/// Process state is a separate projection: a silent process can outlive all
/// 1,500 entries currently loaded in the transcript. Output previews are bounded
/// so active work cannot defeat the forest snapshot's transport budget.
pub(crate) fn current(
    db: &Connection,
    session_id: &str,
) -> Result<Vec<crate::model::SessionEntry>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT e.id,e.session_id,e.parent_entry_id,e.sequence,e.semantic_schema_version,e.kind,e.payload,e.provider_event_id,e.context_visibility,e.token_estimate,e.created_at
         FROM (
           SELECT json_extract(CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,'$.itemId') AS item_id,MAX(sequence) AS sequence
           FROM session_entries WHERE session_id=?1
             AND kind IN ('tool.started','tool.completed','command.started','command.completed')
             AND json_extract(CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,'$.data.backgroundTask')=1
           GROUP BY json_extract(CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,'$.itemId')
         ) latest CROSS JOIN session_entries e ON e.session_id=?1 AND e.sequence=latest.sequence
         WHERE json_extract(CASE WHEN json_valid(e.payload) THEN e.payload ELSE '{}' END,'$.status')='inProgress'
         ORDER BY e.sequence LIMIT 256"
    )?;
    let rows = statement
        .query_map([session_id], |row| {
            let raw: String = row.get(6)?;
            let mut payload: Value = serde_json::from_str(&raw).unwrap_or_default();
            let source = &payload["data"];
            let mut data = serde_json::Map::new();
            for key in [
                "type",
                "sourceCallId",
                "backgroundTask",
                "background",
                "passive",
                "taskId",
                "command",
                "progress",
                "durationMs",
                "outputFile",
                "aggregatedOutput",
            ] {
                if let Some(value) = source.get(key) {
                    data.insert(key.into(), preview(value));
                }
            }
            payload["data"] = Value::Object(data);
            for key in ["text", "title"] {
                if let Some(value) = payload.get(key) {
                    payload[key] = preview(value);
                }
            }
            Ok(crate::model::SessionEntry {
                id: row.get(0)?,
                session_id: row.get(1)?,
                parent_entry_id: row.get(2)?,
                sequence: row.get(3)?,
                semantic_schema_version: row.get(4)?,
                kind: row.get(5)?,
                payload,
                provider_event_id: row.get(7)?,
                context_visibility: row.get(8)?,
                token_estimate: row.get(9)?,
                created_at: row.get(10)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn preview(value: &Value) -> Value {
    match value.as_str() {
        Some(text) if text.len() > 8192 => {
            let mut start = text.len() - 8192;
            while !text.is_char_boundary(start) {
                start += 1;
            }
            Value::String(format!("[earlier content omitted]\n{}", &text[start..]))
        }
        _ => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn active_process_projection_survives_history_windows_and_bounds_output() {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        db.execute("INSERT INTO sessions(id,harness,label,status,metric_source) VALUES('s','claude','Chat','ready','reported')", []).unwrap();
        let mut start = NormalizedEvent::new("tool.started");
        start.item_id = Some("task".into());
        start.status = Some("inProgress".into());
        start.data = json!({"backgroundTask":true,"background":true,"command":"cargo test","aggregatedOutput":"界".repeat(9000)});
        store::session_event(&db, "s", &start, &json!({})).unwrap();
        for _ in 0..5 {
            let mut reply = NormalizedEvent::new("message.completed");
            reply.role = Some("assistant".into());
            reply.text = Some("Later work".into());
            store::session_event(&db, "s", &reply, &json!({})).unwrap();
        }
        assert!(store::session_events_tail(&db, "s", 2)
            .unwrap()
            .iter()
            .all(|event| event.item_id.as_deref() != Some("task")));
        let active = current(&db, "s").unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].payload["data"]["command"], "cargo test");
        assert!(
            active[0].payload["data"]["aggregatedOutput"]
                .as_str()
                .unwrap()
                .len()
                < 8300
        );
        let mut done = start;
        done.kind = "tool.completed".into();
        done.status = Some("completed".into());
        store::session_event(&db, "s", &done, &json!({})).unwrap();
        assert!(current(&db, "s").unwrap().is_empty());
    }

    #[test]
    fn reopening_the_store_closes_tasks_even_when_the_model_was_idle() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("bridge.db");
        {
            let db = store::open(&path).unwrap();
            db.execute("INSERT INTO sessions(id,harness,label,status,metric_source) VALUES('s','claude','Chat','ready','reported')", []).unwrap();
            let mut event = NormalizedEvent::new("tool.started");
            event.item_id = Some("task".into());
            event.status = Some("inProgress".into());
            event.data = json!({"backgroundTask":true,"background":true,"command":"cargo test"});
            store::session_event(&db, "s", &event, &json!({})).unwrap();
        }
        let db = store::open(&path).unwrap();
        assert!(current(&db, "s").unwrap().is_empty());
        assert_eq!(
            store::session_events_tail(&db, "s", 10)
                .unwrap()
                .last()
                .unwrap()
                .status
                .as_deref(),
            Some("unknown")
        );
    }

    #[test]
    fn corrupt_tool_history_does_not_block_activity_reads_or_recovery() {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        db.execute("INSERT INTO sessions(id,harness,label,status,metric_source) VALUES('s','claude','Chat','ready','reported')", []).unwrap();
        let mut event = NormalizedEvent::new("tool.started");
        event.item_id = Some("task".into());
        event.status = Some("inProgress".into());
        event.data = json!({"backgroundTask":true,"background":true});
        store::session_event(&db, "s", &event, &json!({})).unwrap();
        db.execute_batch("DROP TRIGGER session_entries_au_fts")
            .unwrap();
        db.execute(
            "UPDATE session_entries SET payload='corrupt' WHERE session_id='s'",
            [],
        )
        .unwrap();
        assert!(current(&db, "s").unwrap().is_empty());
        assert!(interrupt(&db, None).unwrap().is_empty());
    }

    #[test]
    fn runtime_cleanup_is_scoped_durable_idempotent_and_does_not_invent_success() {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        for session in ["one", "two"] {
            db.execute("INSERT INTO sessions(id,harness,label,status,metric_source) VALUES(?1,'claude','Chat','ready','reported')", [session]).unwrap();
            let mut event = NormalizedEvent::new("tool.started");
            event.item_id = Some("task".into());
            event.status = Some("inProgress".into());
            event.data = json!({"backgroundTask":true,"background":true,"command":"cargo test"});
            store::session_event(&db, session, &event, &json!({})).unwrap();
        }
        let closed = interrupt(&db, Some("one")).unwrap();
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].status.as_deref(), Some("unknown"));
        assert_eq!(closed[0].data["background"], false);
        assert_eq!(closed[0].data["command"], "cargo test");
        assert!(interrupt(&db, Some("one")).unwrap().is_empty());
        assert_eq!(interrupt(&db, None).unwrap().len(), 1);
        assert!(interrupt(&db, None).unwrap().is_empty());
        assert_eq!(
            store::session_events_tail(&db, "two", 10)
                .unwrap()
                .last()
                .unwrap()
                .status
                .as_deref(),
            Some("unknown")
        );
    }
}

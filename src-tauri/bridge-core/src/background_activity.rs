//! Clear claims of live background work when its owning runtime is gone.
use crate::{agent::NormalizedEvent, model::AgentEvent, store, BridgeError};
use rusqlite::{params, Connection, Transaction};
use serde_json::Value;

/// The latest background snapshot of every task, maintained as entries are
/// appended.
///
/// Finding live background work used to mean JSON-parsing every tool and
/// command payload a chat ever recorded: 239 MB for one real chat, on every
/// forest snapshot, every provider start and exit, and (for every chat at
/// once, 3.3 GB) every time the store opened. A task's snapshots are appended
/// like any other entry, so a trigger can keep the latest one per task here,
/// and every reader looks up a handful of rows instead.
///
/// A task is keyed by its `itemId`; snapshots without one share the empty
/// key, the same grouping the old scan produced. Only the entry the row points
/// at decides whether the task is still running, so a stale row can never
/// claim live work: readers re-check the entry's own status.
pub(crate) fn install_index(db: &Transaction<'_>) -> Result<(), BridgeError> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_background_tasks(
           session_id TEXT NOT NULL,
           item_id TEXT NOT NULL,
           sequence INTEGER NOT NULL,
           in_progress INTEGER NOT NULL,
           PRIMARY KEY(session_id,item_id)
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS idx_session_background_tasks_open
           ON session_background_tasks(in_progress,session_id);
         CREATE TRIGGER IF NOT EXISTS session_background_tasks_ai AFTER INSERT ON session_entries
           WHEN NEW.kind IN ('tool.started','tool.completed','command.started','command.completed')
            AND json_valid(NEW.payload)
            AND json_extract(NEW.payload,'$.data.backgroundTask')=1
         BEGIN
           INSERT INTO session_background_tasks(session_id,item_id,sequence,in_progress)
           VALUES(NEW.session_id,
                  COALESCE(json_extract(NEW.payload,'$.itemId'),''),
                  NEW.sequence,
                  json_extract(NEW.payload,'$.status') IS 'inProgress')
           ON CONFLICT(session_id,item_id) DO UPDATE
             SET sequence=excluded.sequence,in_progress=excluded.in_progress
             WHERE excluded.sequence>=session_background_tasks.sequence;
         END;
         CREATE TRIGGER IF NOT EXISTS session_background_tasks_ad AFTER DELETE ON session_entries
           WHEN OLD.kind IN ('tool.started','tool.completed','command.started','command.completed')
         BEGIN
           DELETE FROM session_background_tasks
           WHERE session_id=OLD.session_id AND sequence=OLD.sequence;
         END;",
    )?;
    // One pass over the history that already exists, the same scan every
    // store open used to repeat. Only tasks whose latest snapshot is still
    // running need a row: a missing row and a finished one read the same.
    db.execute(
        "INSERT OR REPLACE INTO session_background_tasks(session_id,item_id,sequence,in_progress)
         SELECT latest.session_id,latest.item_id,latest.sequence,1
         FROM (SELECT session_id,
                      COALESCE(json_extract(CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,'$.itemId'),'') AS item_id,
                      MAX(sequence) AS sequence
               FROM session_entries
               WHERE kind IN ('tool.started','tool.completed','command.started','command.completed')
                 AND json_extract(CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,'$.data.backgroundTask')=1
               GROUP BY session_id,2) latest
         CROSS JOIN session_entries entry
           ON entry.session_id=latest.session_id AND entry.sequence=latest.sequence
         WHERE json_extract(CASE WHEN json_valid(entry.payload) THEN entry.payload ELSE '{}' END,'$.status')='inProgress'",
        [],
    )?;
    Ok(())
}

/// The latest snapshot of every still-running background task, for one
/// session or for all of them, oldest first. `columns` is the entry column
/// list the caller reads.
fn open_task_snapshots_sql(columns: &str) -> String {
    format!(
        "SELECT {columns}
         FROM session_background_tasks task
         CROSS JOIN session_entries e ON e.session_id=task.session_id AND e.sequence=task.sequence
         WHERE task.in_progress=1 AND (?1 IS NULL OR task.session_id=?1)
           AND json_extract(CASE WHEN json_valid(e.payload) THEN e.payload ELSE '{{}}' END,'$.status')='inProgress'
         ORDER BY e.sequence"
    )
}

/// Close the latest task snapshots without inferring an outcome. `None` is
/// startup recovery; a session id is a generation-checked reader exit.
pub(crate) fn interrupt(
    db: &Connection,
    session_id: Option<&str>,
) -> Result<Vec<AgentEvent>, BridgeError> {
    let transaction = db.unchecked_transaction()?;
    let events = interrupt_in_transaction(&transaction, session_id)?;
    transaction.commit()?;
    Ok(events)
}

/// Join a lifecycle change so its task states commit or roll back with it.
pub(crate) fn interrupt_in_transaction(
    db: &Transaction<'_>,
    session_id: Option<&str>,
) -> Result<Vec<AgentEvent>, BridgeError> {
    let mut statement = db.prepare(&open_task_snapshots_sql("e.session_id,e.payload"))?;
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
            store::session_event_in_transaction(
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
    let mut statement = db.prepare(&format!(
        "{} LIMIT 256",
        open_task_snapshots_sql(
            "e.id,e.session_id,e.parent_entry_id,e.sequence,e.semantic_schema_version,e.kind,e.payload,e.provider_event_id,e.context_visibility,e.token_estimate,e.created_at"
        )
    ))?;
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

    fn task_event(item: &str, status: &str) -> NormalizedEvent {
        let mut event = NormalizedEvent::new(if status == "inProgress" { "tool.started" } else { "tool.completed" });
        event.item_id = Some(item.into());
        event.status = Some(status.into());
        event.data = json!({"backgroundTask":true,"background":status == "inProgress","command":"cargo test"});
        event
    }

    fn open_rows(db: &Connection) -> Vec<(String, String, i64)> {
        db.prepare("SELECT session_id,item_id,in_progress FROM session_background_tasks ORDER BY session_id,item_id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    #[test]
    fn the_task_index_follows_appends_without_reading_history() {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        db.execute_batch(
            "INSERT INTO sessions(id,harness,label,status,metric_source) VALUES('s','claude','Chat','ready','reported');
             INSERT INTO sessions(id,harness,label,status,metric_source) VALUES('t','claude','Other','ready','reported');",
        )
        .unwrap();
        store::session_event(&db, "s", &task_event("a", "inProgress"), &json!({})).unwrap();
        store::session_event(&db, "s", &task_event("b", "inProgress"), &json!({})).unwrap();
        store::session_event(&db, "t", &task_event("a", "inProgress"), &json!({})).unwrap();
        // Ordinary tools never get a row.
        let mut plain = NormalizedEvent::new("tool.started");
        plain.item_id = Some("plain".into());
        plain.status = Some("inProgress".into());
        store::session_event(&db, "s", &plain, &json!({})).unwrap();
        assert_eq!(
            open_rows(&db),
            vec![("s".into(), "a".into(), 1), ("s".into(), "b".into(), 1), ("t".into(), "a".into(), 1)]
        );
        store::session_event(&db, "s", &task_event("a", "completed"), &json!({})).unwrap();
        let running: Vec<String> = current(&db, "s")
            .unwrap()
            .into_iter()
            .map(|entry| entry.payload["itemId"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(running, vec!["b"]);
        // Interrupting one session leaves the other's work alone.
        interrupt(&db, Some("s")).unwrap();
        assert!(current(&db, "s").unwrap().is_empty());
        assert_eq!(current(&db, "t").unwrap().len(), 1);
        // Deleting the history drops its rows.
        db.execute_batch("DELETE FROM session_heads WHERE session_id='t'; DELETE FROM session_entries WHERE session_id='t';").unwrap();
        assert!(open_rows(&db).iter().all(|(session, _, _)| session == "s"));
    }

    #[test]
    fn upgrading_closes_tasks_left_running_before_the_index_existed() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("bridge.db");
        {
            let db = store::open(&path).unwrap();
            db.execute("INSERT INTO sessions(id,harness,label,status,metric_source) VALUES('s','claude','Chat','ready','reported')", []).unwrap();
            // History written before the index: no trigger, no table.
            db.execute_batch(
                "DROP TRIGGER session_background_tasks_ai;
                 DROP TRIGGER session_background_tasks_ad;
                 DROP TABLE session_background_tasks;
                 DELETE FROM schema_version WHERE version=77;",
            )
            .unwrap();
            store::session_event(&db, "s", &task_event("done", "inProgress"), &json!({})).unwrap();
            store::session_event(&db, "s", &task_event("done", "completed"), &json!({})).unwrap();
            store::session_event(&db, "s", &task_event("left", "inProgress"), &json!({})).unwrap();
        }
        // The migration backfills what the old full scan would have found, and
        // startup recovery then closes it, exactly as before.
        let db = store::open(&path).unwrap();
        assert!(current(&db, "s").unwrap().is_empty());
        let closed: Vec<(String, String)> = db
            .prepare("SELECT json_extract(payload,'$.itemId'),json_extract(payload,'$.status') FROM session_entries WHERE session_id='s' ORDER BY sequence")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(closed.last().unwrap(), &("left".to_owned(), "unknown".to_owned()));
        assert_eq!(closed.iter().filter(|(_, status)| status == "unknown").count(), 1);
        assert_eq!(open_rows(&db), vec![("s".into(), "left".into(), 0)]);
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

    #[test]
    fn adapter_cleanup_commits_or_rolls_back_with_its_lifecycle_transaction() {
        use crate::session_supervisor::SessionSupervisor;

        for commit in [false, true] {
            let db = store::open(std::path::Path::new(":memory:")).unwrap();
            db.execute("INSERT INTO sessions(id,harness,label,status,metric_source,adapter_pid,adapter_process_identity) VALUES('s','claude','Chat','working','reported',123,'previous-runtime')", []).unwrap();
            let mut event = NormalizedEvent::new("tool.started");
            event.item_id = Some("task".into());
            event.status = Some("inProgress".into());
            event.data = json!({"backgroundTask":true,"background":true,"command":"cargo test"});
            store::session_event(&db, "s", &event, &json!({})).unwrap();

            let transaction = db.unchecked_transaction().unwrap();
            SessionSupervisor::clear_adapter_process_in_transaction(&transaction, "s").unwrap();
            assert!(current(&transaction, "s").unwrap().is_empty());
            let pid: Option<i64> = transaction.query_row("SELECT adapter_pid FROM sessions WHERE id='s'", [], |row| row.get(0)).unwrap();
            assert_eq!(pid, None);
            if commit {
                transaction.commit().unwrap();
            } else {
                transaction.rollback().unwrap();
            }

            assert_eq!(current(&db, "s").unwrap().is_empty(), commit);
            let pid: Option<i64> = db.query_row("SELECT adapter_pid FROM sessions WHERE id='s'", [], |row| row.get(0)).unwrap();
            assert_eq!(pid, if commit { None } else { Some(123) });
            let last = store::session_events_tail(&db, "s", 10).unwrap().pop().unwrap();
            assert_eq!(last.status.as_deref(), Some(if commit { "unknown" } else { "inProgress" }));
        }
    }
}

//! App events: what happened through an app, written into the session forest
//! so memory can learn from it.
//!
//! An event is an `app.event` entry with visibility `hidden`: it is stored,
//! replayed and exported, but never projected into model context and never a
//! recall hit. Memory extraction is the only reader, and only inside the
//! memory scope the app's manifest declares (`memory_extraction::build_digest`).
//!
//! This is the one writer. It refuses what the manifest does not declare, and
//! writes nothing for an app that is off.

use rusqlite::Transaction;
use serde_json::Value;

use super::{builtin, registry};
use crate::BridgeError;

/// The forest entry kind every app event uses.
pub const APP_EVENT_KIND: &str = "app.event";
const VISIBILITY: &str = "hidden";
pub const MAX_SUMMARY_CHARS: usize = 280;
pub const MAX_PAYLOAD_BYTES: usize = 4096;

/// Append one app event to `session_id` inside the caller's transaction.
///
/// `Ok(Some(id))` is the new entry. `Ok(None)` means the app is off, which is
/// a normal state and must not fail the caller. Anything the manifest does not
/// allow is `BridgeError::Invalid` and writes nothing.
pub fn record_tx(
    transaction: &Transaction<'_>,
    session_id: &str,
    app_id: &str,
    event_type: &str,
    summary: &str,
    payload: Value,
) -> Result<Option<String>, BridgeError> {
    let app = builtin(app_id).ok_or_else(|| BridgeError::Invalid(format!("Bridge has no app called {app_id}.")))?;
    let name = &app.manifest.name;
    if !app.manifest.events.iter().any(|declared| declared == event_type) {
        return Err(BridgeError::Invalid(format!("{name} does not record {event_type} events.")));
    }
    let summary = summary.trim();
    if summary.is_empty() || summary.chars().count() > MAX_SUMMARY_CHARS {
        return Err(BridgeError::Invalid(format!(
            "An app event needs a summary of 1 to {MAX_SUMMARY_CHARS} characters."
        )));
    }
    if !payload.is_object() {
        return Err(BridgeError::Invalid("An app event payload must be a JSON object.".into()));
    }
    let payload = scrub(payload);
    if payload.to_string().len() > MAX_PAYLOAD_BYTES {
        return Err(BridgeError::Invalid(format!(
            "An app event payload must be at most {MAX_PAYLOAD_BYTES} bytes."
        )));
    }
    if !registry::is_enabled(transaction, app_id)? {
        return Ok(None);
    }
    let parent: Option<String> = transaction
        .query_row(
            "SELECT active_entry_id FROM session_heads WHERE session_id=?1",
            [session_id],
            |row| row.get(0),
        )
        .or_else(|error| if error == rusqlite::Error::QueryReturnedNoRows { Ok(None) } else { Err(error) })?;
    let entry = crate::store::append_session_entry_tx(
        transaction,
        session_id,
        parent.as_deref(),
        APP_EVENT_KIND,
        &serde_json::json!({
            "appId": app_id,
            "type": event_type,
            "summary": scrub_text(summary),
            "payload": payload,
        }),
        None,
        VISIBILITY,
        Some(0),
    )?;
    Ok(Some(entry.id))
}

fn scrub_text(text: &str) -> String {
    crate::secret_interception::intercept(text).sanitized.text
}

/// Every string, at any depth, keys included.
fn scrub(value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(scrub_text(&text)),
        Value::Array(items) => Value::Array(items.into_iter().map(scrub).collect()),
        Value::Object(map) => Value::Object(map.into_iter().map(|(key, value)| (scrub_text(&key), scrub(value))).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use serde_json::json;
    use std::path::Path;

    fn db() -> Connection {
        let db = crate::store::open(Path::new(":memory:")).unwrap();
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind) VALUES('s1',NULL,'claude','s1','idle','reported','direct')",
            [],
        )
        .unwrap();
        db
    }

    fn record(db: &Connection, app: &str, kind: &str, summary: &str, payload: Value) -> Result<Option<String>, BridgeError> {
        let transaction = db.unchecked_transaction().unwrap();
        let result = record_tx(&transaction, "s1", app, kind, summary, payload);
        transaction.commit().unwrap();
        result
    }

    fn entries(db: &Connection) -> Vec<(String, String, Value)> {
        db.prepare("SELECT kind, context_visibility, payload FROM session_entries WHERE session_id='s1' ORDER BY sequence")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, serde_json::from_str(&row.get::<_, String>(2)?).unwrap())))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    fn records_a_declared_event_as_hidden() {
        let db = db();
        let id = record(&db, "outgoing", "outgoing.sent", "Sent a Slack message to #dev", json!({"family": "slack", "destination": "#dev"}))
            .unwrap()
            .expect("written");
        let rows = entries(&db);
        assert_eq!(rows.len(), 1);
        let (kind, visibility, payload) = &rows[0];
        assert_eq!((kind.as_str(), visibility.as_str()), ("app.event", "hidden"));
        assert_eq!(payload["appId"], "outgoing");
        assert_eq!(payload["type"], "outgoing.sent");
        assert_eq!(payload["summary"], "Sent a Slack message to #dev");
        assert_eq!(payload["payload"], json!({"family": "slack", "destination": "#dev"}));
        let head: String = db.query_row("SELECT active_entry_id FROM session_heads WHERE session_id='s1'", [], |row| row.get(0)).unwrap();
        assert_eq!(head, id);
    }

    #[test]
    fn an_app_event_is_never_context_nor_a_recall_hit() {
        let db = db();
        let forest = crate::session_forest::SessionForest::new(&db);
        forest.append("s1", crate::session_forest::EntryKind::UserMessage, json!({"text": "send the release notes"})).unwrap();
        let id = record(&db, "outgoing", "outgoing.sent", "Sent a Slack message to #zebracrossing", json!({})).unwrap().unwrap();
        let branch = forest.active_branch("s1").unwrap();
        assert!(branch.iter().any(|entry| entry.id == id), "the event is on the branch");
        let projection = crate::context::ContextProjector::project(&branch, 200_000).unwrap();
        assert!(projection.render_entries.iter().all(|entry| entry.id != id), "never projected");
        let indexed: i64 = db.query_row("SELECT count(*) FROM session_entry_fts WHERE entry_id=?1", [&id], |row| row.get(0)).unwrap();
        assert_eq!(indexed, 0, "never a recall hit");
        assert!(crate::session_recall::search(&db, "s1", "zebracrossing", None).unwrap().hits.is_empty());
    }

    #[test]
    fn refuses_undeclared_types_and_unknown_apps() {
        let db = db();
        let undeclared = record(&db, "outgoing", "outgoing.drafted", "Drafted", json!({})).unwrap_err();
        assert_eq!(undeclared.to_string(), BridgeError::Invalid("Outgoing does not record outgoing.drafted events.".into()).to_string());
        let unscoped = record(&db, "pulls", "pulls.opened", "Opened", json!({})).unwrap_err();
        assert!(unscoped.to_string().contains("Pulls does not record pulls.opened events."), "{unscoped}");
        let unknown = record(&db, "linear", "linear.opened", "Opened", json!({})).unwrap_err();
        assert!(unknown.to_string().contains("Bridge has no app called linear."), "{unknown}");
        assert!(entries(&db).is_empty());
    }

    #[test]
    fn refuses_oversized_summary_and_payload() {
        let db = db();
        assert!(record(&db, "outgoing", "outgoing.sent", "", json!({})).is_err());
        assert!(record(&db, "outgoing", "outgoing.sent", "   ", json!({})).is_err());
        assert!(record(&db, "outgoing", "outgoing.sent", &"a".repeat(281), json!({})).is_err());
        assert!(record(&db, "outgoing", "outgoing.sent", "ok", json!(["not", "an", "object"])).is_err());
        assert!(record(&db, "outgoing", "outgoing.sent", "ok", json!({"blob": "x".repeat(4096)})).is_err());
        assert!(entries(&db).is_empty());
        // The limits are inclusive.
        assert!(record(&db, "outgoing", "outgoing.sent", &"a".repeat(280), json!({"blob": "x".repeat(4000)})).unwrap().is_some());
    }

    #[test]
    fn an_off_app_writes_nothing() {
        let db = db();
        registry::set_enabled(&db, "outgoing", false, "now").unwrap();
        assert_eq!(record(&db, "outgoing", "outgoing.sent", "Sent a Slack message to #dev", json!({})).unwrap(), None);
        assert!(entries(&db).is_empty());
        // Off does not excuse an undeclared type: that is a caller bug.
        assert!(record(&db, "outgoing", "outgoing.drafted", "Drafted", json!({})).is_err());
        registry::set_enabled(&db, "outgoing", true, "now").unwrap();
        assert!(record(&db, "outgoing", "outgoing.sent", "Sent a Slack message to #dev", json!({})).unwrap().is_some());
    }

    #[test]
    fn secrets_are_scrubbed_from_summary_and_payload() {
        let db = db();
        let token = "ghp_abcdefghijklmnopqrstuvwxyzABCDEFGHIJ";
        record(
            &db,
            "outgoing",
            "outgoing.sent",
            &format!("Sent {token} to #dev"),
            json!({"destination": format!("#dev {token}"), "nested": [{"value": token}]}),
        )
        .unwrap();
        let stored = entries(&db)[0].2.to_string();
        assert!(!stored.contains(token), "{stored}");
        assert!(stored.contains("#dev"), "{stored}");
    }
}

//! Live context windows: how full each harness's window is, as the harness
//! itself reports it.
//!
//! A chat can run several windows at once (the chat model, an orchestrator,
//! each worker) and replace them on a model switch. Every reading records
//! which harness, model and provider thread it belongs to, so a reading from a
//! window that no longer exists is never shown as the current one.
//!
//! Sources, from most to least direct:
//!
//! * Claude: the sidecar's `context_usage` frame, Claude Code's own `/context`
//!   measurement with a category split (`measured`).
//! * Codex: `thread/tokenUsage/updated`, the last request's `totalTokens`
//!   against `modelContextWindow` (`reported`). The running `total` is the
//!   thread's spend, not its window, and is never used.
//! * ACP agents: `usage_update` `used` / `size` (`reported`).
//! * OpenCode: the last step's token counts; the window is Bridge's catalog
//!   figure, so the reading is `estimated`.
//!
//! A per-turn usage ledger row cannot stand in for any of these: a turn is
//! many requests, and its cache reads sum far past the window.

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{json, Value};

use crate::{model_catalog, BridgeError};

/// How a reading's numbers were obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadingState {
    /// The provider stated used and window tokens.
    Reported,
    /// The harness counted its own window (Claude's `/context`).
    Measured,
    /// At least one figure is Bridge's (a catalog window).
    Estimated,
}

impl ReadingState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reported => "reported",
            Self::Measured => "measured",
            Self::Estimated => "estimated",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "reported" => Self::Reported,
            "measured" => Self::Measured,
            _ => Self::Estimated,
        }
    }
}

/// One observation of a window before it is bound to a session.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextReading {
    pub used_tokens: i64,
    /// `None` when the harness reported no window; recording fills it from
    /// the model catalog and marks the reading estimated.
    pub window_tokens: Option<i64>,
    pub state: ReadingState,
    pub source: &'static str,
    /// Claude only: the sanitized `/context` split.
    pub breakdown: Option<Value>,
}

pub fn install_store(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS context_readings (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL,
            turn_id TEXT,
            harness TEXT NOT NULL,
            model TEXT,
            provider_session_id TEXT,
            used_tokens INTEGER NOT NULL CHECK (used_tokens >= 0),
            window_tokens INTEGER NOT NULL CHECK (window_tokens > 0),
            state TEXT NOT NULL,
            source TEXT NOT NULL,
            breakdown_json TEXT,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS context_readings_session
            ON context_readings(session_id, id);",
    )?;
    Ok(())
}

fn positive(value: Option<&Value>) -> Option<i64> {
    value.and_then(Value::as_i64).filter(|count| *count >= 0)
}

/// The sidecar's `context_usage` frame. The sidecar already capped and
/// sanitized every list; this keeps only the fields the window view reads.
pub fn reading_from_claude_frame(frame: &Value) -> Option<ContextReading> {
    if frame.get("type").and_then(Value::as_str) != Some("context_usage") {
        return None;
    }
    let used_tokens = positive(frame.get("usedTokens"))?;
    let window_tokens = positive(frame.get("windowTokens")).filter(|window| *window > 0)?;
    let breakdown = json!({
        "autoCompactTokens": frame.get("autoCompactTokens").cloned().unwrap_or(Value::Null),
        "autoCompactEnabled": frame.get("autoCompactEnabled").cloned().unwrap_or(Value::Null),
        "categories": frame.get("categories").cloned().unwrap_or_else(|| json!([])),
        "mcpServers": frame.get("mcpServers").cloned().unwrap_or_else(|| json!([])),
        "memoryFiles": frame.get("memoryFiles").cloned().unwrap_or(Value::Null),
        "skills": frame.get("skills").cloned().unwrap_or(Value::Null),
        "agentsTokens": frame.get("agentsTokens").cloned().unwrap_or(Value::Null),
        "messages": frame.get("messages").cloned().unwrap_or(Value::Null),
    });
    Some(ContextReading {
        used_tokens,
        window_tokens: Some(window_tokens),
        state: ReadingState::Measured,
        source: "claude.context_usage",
        breakdown: Some(breakdown),
    })
}

/// A normalized `usage.updated` event from a harness that reports its window
/// through usage. Claude's turn `result` is deliberately not one of them: its
/// figures are the turn's sum, and the sidecar frame carries the window.
pub fn reading_from_usage_event(adapter_id: &str, data: &Value) -> Option<ContextReading> {
    if let Some(last) = data.pointer("/tokenUsage/last") {
        let used_tokens = positive(last.get("totalTokens"))?;
        let window = positive(data.pointer("/tokenUsage/modelContextWindow")).filter(|w| *w > 0);
        return Some(ContextReading {
            used_tokens,
            state: if window.is_some() { ReadingState::Reported } else { ReadingState::Estimated },
            window_tokens: window,
            source: "codex.token_usage",
            breakdown: None,
        });
    }
    if let Some(used_tokens) = positive(data.pointer("/usage/used_tokens")) {
        let window = positive(data.pointer("/usage/context_window")).filter(|w| *w > 0);
        return Some(ContextReading {
            used_tokens,
            state: if window.is_some() { ReadingState::Reported } else { ReadingState::Estimated },
            window_tokens: window,
            source: "acp.usage_update",
            breakdown: None,
        });
    }
    if adapter_id == "opencode" {
        let usage = data.get("usage")?;
        let parts = [
            "input_tokens",
            "output_tokens",
            "reasoning_tokens",
            "cache_read_tokens",
            "cache_write_tokens",
        ]
        .map(|key| positive(usage.get(key)));
        if parts.iter().all(Option::is_none) {
            return None;
        }
        return Some(ContextReading {
            used_tokens: parts.iter().flatten().sum(),
            window_tokens: None,
            state: ReadingState::Estimated,
            source: "opencode.step_tokens",
            breakdown: None,
        });
    }
    None
}

/// Bind a reading to the session as it is now and keep
/// `sessions.context_percent` current. Returns whether a row was written.
pub fn record_reading(
    db: &Connection,
    session_id: &str,
    turn_id: Option<&str>,
    reading: &ContextReading,
) -> Result<bool, BridgeError> {
    let Some((harness, model, provider_session_id)) = db
        .query_row(
            "SELECT harness,model,provider_session_id FROM sessions WHERE id=?1",
            params![session_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()?
    else {
        return Ok(false);
    };
    let (window_tokens, state) = match reading.window_tokens {
        Some(window) => (window, reading.state),
        None => (
            model_catalog::context_window_tokens(&harness, model.as_deref()),
            ReadingState::Estimated,
        ),
    };
    if window_tokens <= 0 || reading.used_tokens < 0 {
        return Ok(false);
    }
    db.execute(
        "INSERT INTO context_readings(session_id,turn_id,harness,model,provider_session_id,used_tokens,window_tokens,state,source,breakdown_json,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            session_id,
            turn_id,
            harness,
            model,
            provider_session_id,
            reading.used_tokens,
            window_tokens,
            state.as_str(),
            reading.source,
            reading.breakdown.as_ref().map(Value::to_string),
            chrono::Utc::now().to_rfc3339(),
        ],
    )?;
    db.execute(
        "UPDATE sessions SET context_percent=?2 WHERE id=?1",
        params![session_id, percent(reading.used_tokens, window_tokens)],
    )?;
    Ok(true)
}

/// Whole percent of the window in use, clamped to `0..=100`.
pub fn percent(used_tokens: i64, window_tokens: i64) -> i64 {
    if window_tokens <= 0 {
        return 0;
    }
    ((used_tokens.max(0) as f64 * 100.0 / window_tokens as f64).round() as i64).clamp(0, 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY, workspace_id TEXT, harness TEXT NOT NULL, label TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'idle', started_at TEXT, model TEXT, provider_session_id TEXT, context_percent INTEGER,
                parent_session_id TEXT, depth INTEGER NOT NULL DEFAULT 0, kind TEXT NOT NULL DEFAULT 'direct');",
        )
        .unwrap();
        let transaction = db.transaction().unwrap();
        install_store(&transaction).unwrap();
        transaction.commit().unwrap();
        db
    }

    fn session(db: &Connection, id: &str, harness: &str, model: &str, thread: &str) {
        db.execute(
            "INSERT INTO sessions(id,harness,model,provider_session_id) VALUES(?1,?2,?3,?4)",
            params![id, harness, model, thread],
        )
        .unwrap();
    }

    fn row(db: &Connection) -> (i64, i64, String, String) {
        db.query_row(
            "SELECT used_tokens,window_tokens,state,source FROM context_readings ORDER BY id DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap()
    }

    #[test]
    fn claude_frame_becomes_a_measured_reading() {
        let reading = reading_from_claude_frame(&json!({
            "type":"context_usage","usedTokens":142_000,"windowTokens":1_000_000,"autoCompactTokens":955_000,
            "categories":[{"name":"Messages","tokens":61_000,"kind":"used"}]
        }))
        .unwrap();
        assert_eq!(reading.state, ReadingState::Measured);
        assert_eq!(reading.window_tokens, Some(1_000_000));
        assert_eq!(reading.breakdown.as_ref().unwrap()["categories"][0]["name"], "Messages");
        assert!(reading_from_claude_frame(&json!({"type":"result"})).is_none());
        assert!(reading_from_claude_frame(&json!({"type":"context_usage","usedTokens":1,"windowTokens":0})).is_none());
    }

    #[test]
    fn codex_usage_uses_last_total_not_running_total() {
        let reading = reading_from_usage_event(
            "codex",
            &json!({"usage":{"input_tokens":80},"tokenUsage":{
                "total":{"totalTokens":900_000},"last":{"totalTokens":90_000},"modelContextWindow":272_000}}),
        )
        .unwrap();
        assert_eq!(reading.used_tokens, 90_000);
        assert_eq!(reading.window_tokens, Some(272_000));
        assert_eq!(reading.state, ReadingState::Reported);
    }

    #[test]
    fn acp_usage_becomes_a_reported_reading() {
        let reading = reading_from_usage_event(
            "cursor",
            &json!({"usage":{"used_tokens":12_000,"context_window":200_000},"context_percent":6}),
        )
        .unwrap();
        assert_eq!((reading.used_tokens, reading.window_tokens), (12_000, Some(200_000)));
        assert_eq!(reading.state, ReadingState::Reported);
    }

    #[test]
    fn opencode_reading_uses_catalog_window_and_is_estimated() {
        let db = db();
        session(&db, "o", "opencode", "google/gemini-2.5-pro", "ses_1");
        let reading = reading_from_usage_event(
            "opencode",
            &json!({"usage":{"input_tokens":4_000,"output_tokens":2_000,"reasoning_tokens":1_000,"cache_read_tokens":3_000,"cache_write_tokens":null}}),
        )
        .unwrap();
        assert_eq!(reading.used_tokens, 10_000);
        assert!(record_reading(&db, "o", None, &reading).unwrap());
        assert_eq!(row(&db), (10_000, 1_000_000, "estimated".into(), "opencode.step_tokens".into()));
        // Claude's per-turn result is never a window reading.
        assert!(reading_from_usage_event("claude", &json!({"usage":{"input_tokens":5}})).is_none());
    }

    #[test]
    fn missing_window_falls_back_to_catalog_as_estimated() {
        let db = db();
        session(&db, "c", "codex", "gpt-5.6", "thread-1");
        let reading = reading_from_usage_event("codex", &json!({"tokenUsage":{"last":{"totalTokens":40_000}}})).unwrap();
        assert!(record_reading(&db, "c", Some("t1"), &reading).unwrap());
        assert_eq!(row(&db), (40_000, 400_000, "estimated".into(), "codex.token_usage".into()));
    }

    #[test]
    fn recording_sets_session_context_percent_clamped() {
        let db = db();
        session(&db, "s", "claude", "claude-opus-5-5", "p1");
        let mut reading = reading_from_claude_frame(&json!({"type":"context_usage","usedTokens":76_000,"windowTokens":200_000})).unwrap();
        record_reading(&db, "s", None, &reading).unwrap();
        let read = |db: &Connection| db.query_row("SELECT context_percent FROM sessions WHERE id='s'", [], |r| r.get::<_, i64>(0)).unwrap();
        assert_eq!(read(&db), 38);
        reading.used_tokens = 250_000;
        record_reading(&db, "s", None, &reading).unwrap();
        assert_eq!(read(&db), 100);
        assert!(!record_reading(&db, "missing", None, &reading).unwrap());
    }
}

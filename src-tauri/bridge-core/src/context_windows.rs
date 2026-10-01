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

use bridge_protocol::messages::{
    ContextBridgeContribution, ContextCompactionOwner, ContextReadingState, ContextSegmentKind,
    ContextWindow, ContextWindowConsumer, ContextWindowForecast, ContextWindowReading,
    ContextWindowRole, ContextWindowSegment, ContextWindowsResult, EarlierContextWindow,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{json, Value};

use crate::{model_catalog, store, BridgeError};

/// Windows beyond this many descendants are not listed; a chat's agent tree
/// is bounded by policy far below it.
const MAX_WINDOWS: usize = 32;
const MAX_EARLIER: usize = 8;
const MAX_CONSUMERS: usize = 6;
/// Turn readings the forecast looks back over, and the fewest it trusts.
const FORECAST_WINDOW: usize = 6;
const FORECAST_MIN_SAMPLES: usize = 3;

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

struct StoredReading {
    turn_id: Option<String>,
    harness: String,
    model: Option<String>,
    provider_session_id: Option<String>,
    used_tokens: i64,
    window_tokens: i64,
    state: ReadingState,
    source: String,
    breakdown: Option<Value>,
    created_at: String,
}

impl StoredReading {
    fn same_thread(&self, harness: &str, model: Option<&str>, thread: Option<&str>) -> bool {
        self.harness == harness
            && self.model.as_deref() == model
            && self.provider_session_id.as_deref() == thread
    }
}

fn wire_state(state: ReadingState) -> ContextReadingState {
    match state {
        ReadingState::Reported => ContextReadingState::Reported,
        ReadingState::Measured => ContextReadingState::Measured,
        ReadingState::Estimated => ContextReadingState::Estimated,
    }
}

fn harness_name(harness: &str) -> String {
    match harness {
        "claude" => "Claude".into(),
        "codex" => "Codex".into(),
        "opencode" => "OpenCode".into(),
        "cursor" => "Cursor".into(),
        "grok" => "Grok".into(),
        other => {
            let mut chars = other.chars();
            chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
        }
    }
}

fn compaction_owner(harness: &str) -> ContextCompactionOwner {
    if matches!(harness, "claude" | "codex" | "opencode") {
        ContextCompactionOwner::Harness
    } else {
        ContextCompactionOwner::Bridge
    }
}

fn readings_for(db: &Connection, session_id: &str) -> Result<Vec<StoredReading>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT turn_id,harness,model,provider_session_id,used_tokens,window_tokens,state,source,breakdown_json,created_at
         FROM context_readings WHERE session_id=?1 ORDER BY id",
    )?;
    let rows = statement.query_map(params![session_id], |row| {
        Ok(StoredReading {
            turn_id: row.get(0)?,
            harness: row.get(1)?,
            model: row.get(2)?,
            provider_session_id: row.get(3)?,
            used_tokens: row.get(4)?,
            window_tokens: row.get(5)?,
            state: ReadingState::parse(&row.get::<_, String>(6)?),
            source: row.get(7)?,
            breakdown: row
                .get::<_, Option<String>>(8)?
                .and_then(|text| serde_json::from_str(&text).ok()),
            created_at: row.get(9)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn segment_kind(value: Option<&str>) -> ContextSegmentKind {
    match value {
        Some("free") => ContextSegmentKind::Free,
        Some("buffer") => ContextSegmentKind::Buffer,
        Some("deferred") => ContextSegmentKind::Deferred,
        _ => ContextSegmentKind::Used,
    }
}

fn segments(breakdown: Option<&Value>) -> Vec<ContextWindowSegment> {
    let mut segments: Vec<ContextWindowSegment> = breakdown
        .and_then(|value| value.get("categories"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|category| {
            Some(ContextWindowSegment {
                name: category.get("name")?.as_str()?.to_owned(),
                tokens: positive(category.get("tokens")).unwrap_or(0),
                kind: segment_kind(category.get("kind").and_then(Value::as_str)),
            })
        })
        .collect();
    segments.sort_by(|left, right| right.tokens.cmp(&left.tokens));
    segments
}

fn consumers(breakdown: Option<&Value>) -> Vec<ContextWindowConsumer> {
    let Some(breakdown) = breakdown else {
        return vec![];
    };
    let tools = breakdown
        .pointer("/messages/toolsByType")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tool| {
            Some(ContextWindowConsumer {
                label: format!("{} calls and results", tool.get("name")?.as_str()?),
                tokens: positive(tool.get("tokens"))?,
                detail: None,
            })
        });
    let servers = breakdown
        .get("mcpServers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|server| {
            let tools = positive(server.get("tools")).unwrap_or(0);
            Some(ContextWindowConsumer {
                label: format!("MCP · {}", server.get("name")?.as_str()?),
                tokens: positive(server.get("tokens"))?,
                detail: Some(format!(
                    "{tools} tool{} · every turn",
                    if tools == 1 { "" } else { "s" }
                )),
            })
        });
    let mut all: Vec<_> = tools.chain(servers).filter(|consumer| consumer.tokens > 0).collect();
    all.sort_by(|left, right| right.tokens.cmp(&left.tokens));
    all.truncate(MAX_CONSUMERS);
    all
}

/// The last reading of each turn on the current thread, oldest first. A
/// harness may report several requests per turn; the turn's end is the one
/// that the next turn grows from.
fn forecast(thread: &[&StoredReading], target_tokens: i64) -> Option<ContextWindowForecast> {
    let mut turns: Vec<&StoredReading> = Vec::new();
    for reading in thread {
        match turns.last_mut() {
            Some(last) if last.turn_id.is_some() && last.turn_id == reading.turn_id => *last = reading,
            _ => turns.push(reading),
        }
    }
    let recent = &turns[turns.len().saturating_sub(FORECAST_WINDOW)..];
    if recent.len() < FORECAST_MIN_SAMPLES {
        return None;
    }
    let first = recent.first()?.used_tokens;
    let last = recent.last()?.used_tokens;
    let growth_per_turn = (last - first) / (recent.len() as i64 - 1);
    if growth_per_turn <= 0 {
        return None;
    }
    let remaining = (target_tokens - last).max(0);
    Some(ContextWindowForecast {
        growth_per_turn,
        turns_remaining: (remaining + growth_per_turn - 1) / growth_per_turn,
        samples: recent.len() as u32,
    })
}

struct SessionRow {
    id: String,
    label: String,
    kind: String,
    harness: String,
    model: Option<String>,
    status: String,
    depth: i64,
    provider_session_id: Option<String>,
}

fn session_tree(db: &Connection, session_id: &str) -> Result<Vec<SessionRow>, BridgeError> {
    let mut statement = db.prepare(
        "WITH RECURSIVE tree(id, level) AS (
             SELECT id, 0 FROM sessions WHERE id=?1
             UNION ALL
             SELECT s.id, tree.level + 1 FROM sessions s JOIN tree ON s.parent_session_id=tree.id
             WHERE tree.level < 8
         )
         SELECT s.id,s.label,s.kind,s.harness,s.model,s.status,COALESCE(s.depth,0),s.provider_session_id
         FROM tree JOIN sessions s ON s.id=tree.id
         ORDER BY tree.level, s.started_at IS NULL, s.started_at, s.rowid
         LIMIT ?2",
    )?;
    let rows = statement.query_map(params![session_id, MAX_WINDOWS as i64], |row| {
        Ok(SessionRow {
            id: row.get(0)?,
            label: row.get(1)?,
            kind: row.get(2)?,
            harness: row.get(3)?,
            model: row.get(4)?,
            status: row.get(5)?,
            depth: row.get(6)?,
            provider_session_id: row.get(7)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn window(db: &Connection, session: SessionRow) -> Result<(ContextWindow, Vec<StoredReading>), BridgeError> {
    let readings = readings_for(db, &session.id)?;
    let thread: Vec<&StoredReading> = readings
        .iter()
        .filter(|reading| {
            reading.same_thread(
                &session.harness,
                session.model.as_deref(),
                session.provider_session_id.as_deref(),
            )
        })
        .collect();
    let role = if session.depth > 0 {
        ContextWindowRole::Worker
    } else if session.kind == "orchestrator" {
        ContextWindowRole::Orchestrator
    } else {
        ContextWindowRole::Chat
    };
    let current = thread.last().map(|latest| {
        let auto_compact_tokens = latest
            .breakdown
            .as_ref()
            .filter(|breakdown| breakdown.get("autoCompactEnabled") != Some(&Value::Bool(false)))
            .and_then(|breakdown| positive(breakdown.get("autoCompactTokens")))
            .filter(|tokens| *tokens > 0);
        ContextWindowReading {
            used_tokens: latest.used_tokens,
            window_tokens: latest.window_tokens,
            percent: percent(latest.used_tokens, latest.window_tokens),
            state: wire_state(latest.state),
            source: latest.source.clone(),
            observed_at: latest.created_at.clone(),
            turn_id: latest.turn_id.clone(),
            auto_compact_tokens,
            compaction_owner: compaction_owner(&session.harness),
            segments: segments(latest.breakdown.as_ref()),
            consumers: consumers(latest.breakdown.as_ref()),
            forecast: forecast(&thread, auto_compact_tokens.unwrap_or(latest.window_tokens)),
        }
    });
    let unavailable_reason = current.is_none().then(|| {
        let name = harness_name(&session.harness);
        if readings.is_empty() {
            if session.provider_session_id.is_none() {
                format!("{name} starts reporting after its first reply.")
            } else {
                format!("{name} has not reported its context window in this chat.")
            }
        } else {
            "No reading yet since this model started.".to_owned()
        }
    });
    Ok((
        ContextWindow {
            session_id: session.id,
            label: session.label,
            kind: session.kind,
            role,
            harness: session.harness,
            model: session.model,
            status: session.status,
            depth: session.depth,
            current,
            unavailable_reason,
        },
        readings,
    ))
}

/// Every live window in the chat's agent tree, the chat's replaced windows,
/// and Bridge's own share of the prompt.
pub fn context_windows(db: &Connection, session_id: &str) -> Result<ContextWindowsResult, BridgeError> {
    let tree = session_tree(db, session_id)?;
    if tree.is_empty() {
        return Err(BridgeError::Invalid(format!("unknown session: {session_id}")));
    }
    let mut windows = Vec::with_capacity(tree.len());
    let mut earlier = Vec::new();
    for (index, session) in tree.into_iter().enumerate() {
        let current_thread = (
            session.harness.clone(),
            session.model.clone(),
            session.provider_session_id.clone(),
        );
        let (window, readings) = window(db, session)?;
        if index == 0 {
            // The last reading of each thread this chat has since replaced.
            let mut seen = vec![current_thread];
            for reading in readings.iter().rev() {
                let thread = (
                    reading.harness.clone(),
                    reading.model.clone(),
                    reading.provider_session_id.clone(),
                );
                if seen.contains(&thread) {
                    continue;
                }
                seen.push(thread);
                earlier.push(EarlierContextWindow {
                    harness: reading.harness.clone(),
                    model: reading.model.clone(),
                    used_tokens: reading.used_tokens,
                    window_tokens: reading.window_tokens,
                    percent: percent(reading.used_tokens, reading.window_tokens),
                    state: wire_state(reading.state),
                    observed_at: reading.created_at.clone(),
                });
                if earlier.len() == MAX_EARLIER {
                    break;
                }
            }
        }
        windows.push(window);
    }
    let bridge = store::latest_prompt_compilation(db, session_id)?
        .filter(|record| {
            record.stable_token_estimate.is_some() || record.variable_token_estimate.is_some()
        })
        .map(|record| ContextBridgeContribution {
            stable_tokens: record.stable_token_estimate,
            variable_tokens: record.variable_token_estimate,
            method: record.token_estimate_source,
        });
    Ok(ContextWindowsResult {
        session_id: session_id.to_owned(),
        windows,
        earlier,
        bridge,
    })
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    /// A result exercising every optional field, for the protocol mirror.
    pub fn sample_result() -> ContextWindowsResult {
        ContextWindowsResult {
            session_id: "s-1".into(),
            windows: vec![
                ContextWindow {
                    session_id: "s-1".into(),
                    label: "Chat".into(),
                    kind: "direct".into(),
                    role: ContextWindowRole::Chat,
                    harness: "claude".into(),
                    model: Some("claude-opus-5-5".into()),
                    status: "ready".into(),
                    depth: 0,
                    current: Some(ContextWindowReading {
                        used_tokens: 142_000,
                        window_tokens: 1_000_000,
                        percent: 14,
                        state: ContextReadingState::Measured,
                        source: "claude.context_usage".into(),
                        observed_at: "2026-10-01T00:00:00Z".into(),
                        turn_id: Some("t-1".into()),
                        auto_compact_tokens: Some(955_000),
                        compaction_owner: ContextCompactionOwner::Harness,
                        segments: vec![ContextWindowSegment {
                            name: "Messages".into(),
                            tokens: 61_000,
                            kind: ContextSegmentKind::Used,
                        }],
                        consumers: vec![ContextWindowConsumer {
                            label: "MCP · railway".into(),
                            tokens: 9_000,
                            detail: Some("47 tools · every turn".into()),
                        }],
                        forecast: Some(ContextWindowForecast {
                            growth_per_turn: 11_000,
                            turns_remaining: 74,
                            samples: 6,
                        }),
                    }),
                    unavailable_reason: None,
                },
                ContextWindow {
                    session_id: "w-1".into(),
                    label: "Worker".into(),
                    kind: "worker".into(),
                    role: ContextWindowRole::Worker,
                    harness: "cursor".into(),
                    model: None,
                    status: "working".into(),
                    depth: 1,
                    current: None,
                    unavailable_reason: Some("Cursor starts reporting after its first reply.".into()),
                },
            ],
            earlier: vec![EarlierContextWindow {
                harness: "claude".into(),
                model: Some("claude-sonnet-5-5".into()),
                used_tokens: 52_000,
                window_tokens: 200_000,
                percent: 26,
                state: ContextReadingState::Estimated,
                observed_at: "2026-09-30T00:00:00Z".into(),
            }],
            bridge: Some(ContextBridgeContribution {
                stable_tokens: Some(3_200),
                variable_tokens: None,
                method: Some("chars/4".into()),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let scratch = tempfile::tempdir().unwrap();
        let db = store::open(&scratch.path().join("bridge.db")).unwrap();
        // The connection keeps the file open; the directory can go.
        std::mem::forget(scratch);
        db
    }

    fn session(db: &Connection, id: &str, harness: &str, model: &str, thread: &str) {
        child(db, id, harness, model, thread, None);
    }

    fn child(db: &Connection, id: &str, harness: &str, model: &str, thread: &str, parent: Option<&str>) {
        db.execute(
            "INSERT INTO sessions(id,harness,label,status,metric_source,kind,depth,model,provider_session_id,parent_session_id,started_at)
             VALUES(?1,?2,?1,'ready','reported',?3,?4,?5,NULLIF(?6,''),?7,?8)",
            params![
                id,
                harness,
                if parent.is_some() { "worker" } else { "direct" },
                i64::from(parent.is_some()),
                model,
                thread,
                parent,
                format!("2026-10-01T00:00:0{}Z", id.len() % 10),
            ],
        )
        .unwrap();
    }

    fn read(db: &Connection, id: &str, turn: &str, used: i64, window: i64) {
        let reading = ContextReading {
            used_tokens: used,
            window_tokens: Some(window),
            state: ReadingState::Reported,
            source: "test",
            breakdown: None,
        };
        assert!(record_reading(db, id, Some(turn), &reading).unwrap());
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

    #[test]
    fn windows_include_descendants_in_depth_order() {
        let db = db();
        session(&db, "chat", "claude", "claude-opus-5-5", "p1");
        child(&db, "w1", "claude", "claude-sonnet-5-5", "p2", Some("chat"));
        child(&db, "w22", "codex", "gpt-5.6", "t1", Some("chat"));
        child(&db, "w1-sub", "codex", "gpt-5.6", "t2", Some("w1"));
        session(&db, "unrelated", "codex", "gpt-5.6", "t3");
        read(&db, "w1", "a", 72_000, 200_000);
        let result = context_windows(&db, "chat").unwrap();
        let ids: Vec<_> = result.windows.iter().map(|w| w.session_id.as_str()).collect();
        assert_eq!(ids, ["chat", "w1", "w22", "w1-sub"]);
        assert_eq!(result.windows[0].role, ContextWindowRole::Chat);
        assert_eq!(result.windows[1].role, ContextWindowRole::Worker);
        assert_eq!(result.windows[1].current.as_ref().unwrap().percent, 36);
        assert_eq!(result.windows[1].current.as_ref().unwrap().compaction_owner, ContextCompactionOwner::Harness);
        assert!(result.windows[2].current.is_none());
        assert!(result.windows[2].unavailable_reason.is_some());
    }

    #[test]
    fn reading_after_model_switch_is_not_current() {
        let db = db();
        session(&db, "chat", "claude", "claude-sonnet-5-5", "p1");
        read(&db, "chat", "t1", 52_000, 200_000);
        db.execute("UPDATE sessions SET model='claude-opus-5-5',provider_session_id=NULL WHERE id='chat'", []).unwrap();
        let result = context_windows(&db, "chat").unwrap();
        let chat = &result.windows[0];
        assert!(chat.current.is_none(), "the Sonnet reading must not describe the Opus window");
        assert_eq!(chat.unavailable_reason.as_deref(), Some("No reading yet since this model started."));
        assert_eq!(result.earlier.len(), 1);
        assert_eq!(result.earlier[0].model.as_deref(), Some("claude-sonnet-5-5"));
        assert_eq!(result.earlier[0].percent, 26);

        db.execute("UPDATE sessions SET provider_session_id='p2' WHERE id='chat'", []).unwrap();
        read(&db, "chat", "t2", 61_000, 1_000_000);
        let result = context_windows(&db, "chat").unwrap();
        assert_eq!(result.windows[0].current.as_ref().unwrap().used_tokens, 61_000);
        assert_eq!(result.earlier.len(), 1, "the current thread is not listed as earlier");
    }

    #[test]
    fn forecast_needs_three_samples_and_positive_growth() {
        let db = db();
        session(&db, "chat", "codex", "gpt-5.6", "t");
        read(&db, "chat", "a", 10_000, 100_000);
        read(&db, "chat", "b", 20_000, 100_000);
        assert!(context_windows(&db, "chat").unwrap().windows[0].current.as_ref().unwrap().forecast.is_none());
        // Several requests in one turn count as that turn's last reading.
        read(&db, "chat", "c", 25_000, 100_000);
        read(&db, "chat", "c", 30_000, 100_000);
        let forecast = context_windows(&db, "chat").unwrap().windows[0].current.clone().unwrap().forecast.unwrap();
        assert_eq!(forecast.samples, 3);
        assert_eq!(forecast.growth_per_turn, 10_000);
        assert_eq!(forecast.turns_remaining, 7);
        read(&db, "chat", "d", 5_000, 100_000);
        read(&db, "chat", "e", 4_000, 100_000);
        assert!(context_windows(&db, "chat").unwrap().windows[0].current.as_ref().unwrap().forecast.is_none(),
            "a compaction makes growth non-positive");
    }

    #[test]
    fn claude_breakdown_feeds_segments_consumers_and_auto_compact() {
        let db = db();
        session(&db, "chat", "claude", "claude-opus-5-5", "p");
        let reading = reading_from_claude_frame(&json!({
            "type":"context_usage","usedTokens":142_000,"windowTokens":1_000_000,"autoCompactTokens":955_000,"autoCompactEnabled":true,
            "categories":[{"name":"Messages","tokens":61_000,"kind":"used"},{"name":"Free space","tokens":858_000,"kind":"free"}],
            "mcpServers":[{"name":"railway","tokens":9_000,"tools":47}],
            "messages":{"toolsByType":[{"name":"Bash","tokens":21_000}]}
        })).unwrap();
        record_reading(&db, "chat", None, &reading).unwrap();
        let current = context_windows(&db, "chat").unwrap().windows[0].current.clone().unwrap();
        assert_eq!(current.state, ContextReadingState::Measured);
        assert_eq!(current.auto_compact_tokens, Some(955_000));
        assert_eq!(current.segments[0].kind, ContextSegmentKind::Free);
        assert_eq!(current.segments[1].name, "Messages");
        assert_eq!(current.consumers[0].label, "Bash calls and results");
        assert_eq!(current.consumers[1].label, "MCP · railway");
        assert_eq!(current.consumers[1].detail.as_deref(), Some("47 tools · every turn"));
    }

    #[test]
    fn unknown_session_is_an_error() {
        assert!(context_windows(&db(), "missing").is_err());
    }

    #[test]
    fn cursor_window_reports_unavailable_reason() {
        let db = db();
        session(&db, "chat", "cursor", "auto", "");
        let window = &context_windows(&db, "chat").unwrap().windows[0];
        assert!(window.current.is_none());
        assert_eq!(window.unavailable_reason.as_deref(), Some("Cursor starts reporting after its first reply."));
        db.execute("UPDATE sessions SET provider_session_id='c1' WHERE id='chat'", []).unwrap();
        let window = &context_windows(&db, "chat").unwrap().windows[0];
        assert_eq!(window.unavailable_reason.as_deref(), Some("Cursor has not reported its context window in this chat."));
        assert_eq!(compaction_owner("cursor"), ContextCompactionOwner::Bridge);
    }
}

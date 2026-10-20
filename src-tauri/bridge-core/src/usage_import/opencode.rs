//! OpenCode: `~/.local/share/opencode/opencode.db`.
//!
//! Assistant messages store their metadata as JSON in `message.data`:
//! `tokens`, `cost`, `modelID`, `providerID`, and timestamps. Message text
//! lives in a separate `part` table that this importer never touches. The
//! database is opened read-only; the cursor is the `(time_updated, id)`
//! high-water mark, so an edited message is seen again by its id and
//! upserted, replacing whatever was imported from it earlier (e.g. a
//! preliminary row seen before the assistant turn finished).
//!
//! OpenCode 2.x writes new history to `session_message` (sessions in
//! `session_v2`) and leaves `message` as it was. It migrates 1.x history
//! across under the same ids, so the 2.x pass skips any id `message` still
//! holds; every message is read once, from the table that wrote it.

use super::scan::{finish_source, persist_records};
use super::{
    json_count, location_fingerprint, microusd_from_usd, millis_to_rfc3339, ParsedUsage, SourceEnv,
};
use crate::analytics::{
    AnalyticsScanResult, CoverageState, DiscoveredAnalyticsSource, ExactTotalFormula,
    ImporterCapability, NumericUsagePayload, TokenUsage,
};
use crate::BridgeError;
use rusqlite::{params, Connection, OpenFlags, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

pub const AGENT: &str = "opencode";
pub const PROVIDER: &str = "opencode";

pub fn discover(env: &SourceEnv) -> DiscoveredAnalyticsSource {
    let location = env.opencode_db_path();
    let (coverage, reason) = if location.is_file() {
        (CoverageState::Partial, None)
    } else {
        (
            CoverageState::Empty,
            Some("OpenCode database not found".to_string()),
        )
    };
    DiscoveredAnalyticsSource {
        agent: AGENT.into(),
        provider: PROVIDER.into(),
        location_fingerprint: location_fingerprint(AGENT, &location),
        location,
        detected_version: None,
        capability: ImporterCapability::Supported,
        coverage,
        reason,
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OpenCodeCursor {
    pub time_updated: i64,
    pub id: String,
    /// The same high-water mark over 2.x's `session_message`. Absent from
    /// cursors saved before 2.x was read, which then start it from zero.
    #[serde(default)]
    pub v2_time_updated: i64,
    #[serde(default)]
    pub v2_id: String,
}

/// Opens the database without ever writing to it. A plain read-only open
/// works while the WAL's shared-memory file exists; when it does not,
/// `immutable=1` reads the main file as-is.
pub(crate) fn open_read_only(path: &Path) -> Result<Connection, BridgeError> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    match Connection::open_with_flags(path, flags) {
        Ok(db) => Ok(db),
        Err(first) => {
            let uri = format!("file:{}?mode=ro&immutable=1", path.to_string_lossy());
            Connection::open_with_flags(uri, flags | OpenFlags::SQLITE_OPEN_URI)
                .map_err(|_| BridgeError::Db(first))
        }
    }
}

struct MessageRow {
    id: String,
    session_id: String,
    time_created: i64,
    time_updated: i64,
    data: String,
    parent_session_id: Option<String>,
    directory: Option<String>,
}

pub(crate) fn scan(
    tx: &Transaction<'_>,
    source_id: &str,
    source: &DiscoveredAnalyticsSource,
    cursor_text: Option<&str>,
    max_records: usize,
    _env: &SourceEnv,
) -> Result<AnalyticsScanResult, BridgeError> {
    let mut cursor: OpenCodeCursor = cursor_text
        .and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or_default();
    let (rows, v2_rows) = {
        let history = open_read_only(&source.location)?;
        read_rows(&history, &cursor, max_records)?
    };
    let truncated = rows.len() + v2_rows.len() >= max_records;
    let mut skipped = 0_usize;
    let mut records = Vec::with_capacity(rows.len() + v2_rows.len());
    for row in rows {
        cursor.time_updated = row.time_updated;
        cursor.id = row.id.clone();
        match parse_message(&row) {
            Some(record) => records.push((record, String::new())),
            None => skipped += 1,
        }
    }
    for row in v2_rows {
        cursor.v2_time_updated = row.time_updated;
        cursor.v2_id = row.id.clone();
        match parse_v2_message(&row) {
            Some(record) => records.push((record, String::new())),
            None => skipped += 1,
        }
    }
    let persisted = persist_records(tx, source_id, AGENT, records, true)?;
    skipped += persisted.ignored;
    let coverage = if truncated {
        CoverageState::Partial
    } else {
        CoverageState::Complete
    };
    let cursor_json =
        serde_json::to_string(&cursor).map_err(|error| BridgeError::Invalid(error.to_string()))?;
    finish_source(
        tx,
        source_id,
        Some(&cursor_json),
        coverage,
        None,
        Some((persisted.observations.len(), skipped)),
    )?;
    Ok(AnalyticsScanResult {
        records_imported: persisted.observations.len(),
        observations: persisted.observations,
        attributions: vec![],
        next_cursor: Some(cursor_json),
        records_skipped: skipped,
        coverage,
        warning: None,
    })
}

fn has_table(history: &Connection, name: &str) -> Result<bool, BridgeError> {
    Ok(history
        .prepare("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1")?
        .exists(params![name])?)
}

fn query_rows(
    history: &Connection,
    sql: &str,
    time_updated: i64,
    id: &str,
    limit: usize,
) -> Result<Vec<MessageRow>, BridgeError> {
    let mut statement = history.prepare(sql)?;
    let rows = statement.query_map(params![time_updated, id, limit as i64], |row| {
        Ok(MessageRow {
            id: row.get(0)?,
            session_id: row.get(1)?,
            time_created: row.get(2)?,
            time_updated: row.get(3)?,
            data: row.get(4)?,
            parent_session_id: row.get(5)?,
            directory: row.get(6)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// 1.x rows first, then 2.x rows with whatever budget is left. Either table
/// may be absent: an old install never had `session_message`, and a future
/// one may drop `message`.
fn read_rows(
    history: &Connection,
    cursor: &OpenCodeCursor,
    max_records: usize,
) -> Result<(Vec<MessageRow>, Vec<MessageRow>), BridgeError> {
    let has_legacy = has_table(history, "message")?;
    let rows = if has_legacy {
        query_rows(
            history,
            "SELECT m.id, m.session_id, m.time_created, m.time_updated, m.data, s.parent_id, s.directory
             FROM message m LEFT JOIN session s ON s.id = m.session_id
             WHERE (m.time_updated > ?1 OR (m.time_updated = ?1 AND m.id > ?2))
               AND m.data LIKE '%\"role\":\"assistant\"%'
             ORDER BY m.time_updated, m.id
             LIMIT ?3",
            cursor.time_updated,
            &cursor.id,
            max_records,
        )?
    } else {
        Vec::new()
    };
    let budget = max_records.saturating_sub(rows.len());
    if budget == 0 || !has_table(history, "session_message")? || !has_table(history, "session_v2")? {
        return Ok((rows, Vec::new()));
    }
    let migrated = if has_legacy {
        "AND NOT EXISTS (SELECT 1 FROM message legacy WHERE legacy.id = m.id)"
    } else {
        ""
    };
    let v2_rows = query_rows(
        history,
        &format!(
            "SELECT m.id, m.session_id, m.time_created, m.time_updated, m.data, s.parent_id, s.directory
             FROM session_message m LEFT JOIN session_v2 s ON s.id = m.session_id
             WHERE m.type = 'assistant'
               AND (m.time_updated > ?1 OR (m.time_updated = ?1 AND m.id > ?2))
               {migrated}
             ORDER BY m.time_updated, m.id
             LIMIT ?3"
        ),
        cursor.v2_time_updated,
        &cursor.v2_id,
        budget,
    )?;
    Ok((rows, v2_rows))
}

/// A 2.x assistant message names its model as `{id, providerID}` and counts
/// tokens without cache in `input`; restated in 1.x terms it parses the same.
fn parse_v2_message(row: &MessageRow) -> Option<ParsedUsage> {
    let data: Value = serde_json::from_str(&row.data).ok()?;
    let tokens = crate::opencode_v2::inclusive_tokens(data.get("tokens").filter(|t| t.is_object())?);
    let legacy = serde_json::json!({
        "role": "assistant",
        "modelID": data.pointer("/model/id"),
        "providerID": data.pointer("/model/providerID"),
        "cost": data.get("cost"),
        "time": data.get("time"),
    });
    let record = parse_assistant_data(&legacy, &tokens, row)?;
    record.has_tokens().then_some(record)
}

fn parse_message(row: &MessageRow) -> Option<ParsedUsage> {
    let data: Value = serde_json::from_str(&row.data).ok()?;
    if data.get("role").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    let tokens = data.get("tokens").filter(|t| t.is_object())?;
    let record = parse_assistant_data(&data, tokens, row)?;
    record.has_tokens().then_some(record)
}

fn parse_assistant_data(data: &Value, tokens: &Value, row: &MessageRow) -> Option<ParsedUsage> {
    let input = json_count(tokens.get("input"));
    let output = json_count(tokens.get("output"));
    let reasoning_raw = json_count(tokens.get("reasoning"));
    let total = json_count(tokens.get("total"));
    let cache = tokens.get("cache");
    let cache_read = cache.and_then(|c| json_count(c.get("read")));
    let cache_write = cache.and_then(|c| json_count(c.get("write")));
    let uncached =
        input.map(|input| (input - cache_read.unwrap_or(0) - cache_write.unwrap_or(0)).max(0));
    let reasoning = match (reasoning_raw, output) {
        (Some(r), Some(o)) => Some(r.min(o)),
        (r, None) => r,
        (None, Some(_)) => None,
    };
    let mut numeric = BTreeMap::new();
    for (key, value) in [
        ("input_tokens", input),
        ("cache_read_input_tokens", cache_read),
        ("cache_creation_input_tokens", cache_write),
        ("output_tokens", output),
        ("reasoning_tokens", reasoning_raw),
        ("total_tokens", total),
    ] {
        if let Some(value) = value {
            numeric.insert(key.to_string(), value);
        }
    }
    let numeric_usage = NumericUsagePayload::new(numeric).ok()?;
    let occurred_ms = data
        .get("time")
        .and_then(|t| t.get("created"))
        .and_then(Value::as_i64)
        .unwrap_or(row.time_created);
    let occurred_at = millis_to_rfc3339(occurred_ms)?;
    let provider = data
        .get("providerID")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
        .unwrap_or(PROVIDER)
        .to_string();
    let exact_total_formula = if total.is_some() {
        ExactTotalFormula::ProviderReported
    } else {
        ExactTotalFormula::InputIncludesCachePlusOutput
    };
    Some(ParsedUsage {
        native_record_id: row.id.clone(),
        native_session_id: Some(row.session_id.clone()),
        parent_native_session_id: row.parent_session_id.clone(),
        session_type: row.parent_session_id.as_ref().map(|_| "child".to_string()),
        occurred_at,
        model: data
            .get("modelID")
            .and_then(Value::as_str)
            .filter(|m| !m.is_empty())
            .map(str::to_string),
        provider,
        input_semantics: "inclusive",
        output_semantics: "delta",
        usage: TokenUsage {
            total_input_tokens: input,
            uncached_input_tokens: uncached,
            cache_read_tokens: cache_read,
            cache_write_tokens: cache_write,
            output_tokens: output,
            reasoning_tokens: reasoning,
            tool_use_tokens: None,
            provider_reported_total_tokens: total,
            exact_total_formula,
        },
        numeric_usage,
        reported_cost_microusd: data
            .get("cost")
            .and_then(Value::as_f64)
            .and_then(microusd_from_usd),
        project_path: row.directory.clone(),
    })
}

#[cfg(test)]
pub(crate) mod fixtures {
    use rusqlite::{params, Connection};
    use std::path::Path;

    /// Builds a database with OpenCode's `session` and `message` tables and
    /// nothing else the importer would need.
    pub fn create_db(path: &Path) -> Connection {
        let db = Connection::open(path).unwrap();
        db.execute_batch(
            "CREATE TABLE session (id text PRIMARY KEY, project_id text NOT NULL, parent_id text, slug text NOT NULL, directory text NOT NULL, title text NOT NULL, version text NOT NULL, time_created integer NOT NULL, time_updated integer NOT NULL);
             CREATE TABLE message (id text PRIMARY KEY, session_id text NOT NULL, time_created integer NOT NULL, time_updated integer NOT NULL, data text NOT NULL);
             CREATE TABLE part (id text PRIMARY KEY, message_id text NOT NULL, data text NOT NULL);",
        )
        .unwrap();
        db
    }

    /// Adds 2.x's `session_v2` and `session_message` beside the 1.x tables.
    pub fn add_v2_tables(db: &Connection) {
        db.execute_batch(
            "CREATE TABLE session_v2 (id text PRIMARY KEY, project_id text NOT NULL, parent_id text, slug text NOT NULL, directory text NOT NULL, title text, version text NOT NULL, time_created integer NOT NULL, time_updated integer NOT NULL);
             CREATE TABLE session_message (id text PRIMARY KEY, session_id text NOT NULL, type text NOT NULL, seq integer NOT NULL, time_created integer NOT NULL, time_updated integer NOT NULL, data text NOT NULL);",
        )
        .unwrap();
    }

    pub fn insert_v2_message(db: &Connection, id: &str, session: &str, kind: &str, time_updated: i64, data: &str) {
        db.execute(
            "INSERT INTO session_message(id,session_id,type,seq,time_created,time_updated,data) VALUES(?1,?2,?3,?4,?5,?5,?6)",
            params![id, session, kind, time_updated, time_updated, data],
        )
        .unwrap();
    }

    pub fn insert_session(db: &Connection, id: &str, parent: Option<&str>, directory: &str) {
        db.execute(
            "INSERT INTO session(id,project_id,parent_id,slug,directory,title,version,time_created,time_updated) VALUES(?1,'proj',?2,'slug',?3,'SECRET TITLE','1.0',1,1)",
            params![id, parent, directory],
        )
        .unwrap();
    }

    pub fn assistant_data(
        model: &str,
        provider: &str,
        input: i64,
        output: i64,
        reasoning: i64,
        cache_read: i64,
        cache_write: i64,
        cost: f64,
        created_ms: i64,
    ) -> String {
        let total = input + output + reasoning;
        format!(
            r#"{{"parentID":"msg_user","role":"assistant","mode":"build","agent":"build","path":{{"cwd":"/Users/x/voicey","root":"/Users/x/voicey"}},"cost":{cost},"tokens":{{"total":{total},"input":{input},"output":{output},"reasoning":{reasoning},"cache":{{"write":{cache_write},"read":{cache_read}}}}},"modelID":"{model}","providerID":"{provider}","time":{{"created":{created_ms},"completed":{}}},"finish":"tool-calls"}}"#,
            created_ms + 15_000
        )
    }

    pub fn insert_message(db: &Connection, id: &str, session: &str, time_updated: i64, data: &str) {
        db.execute(
            "INSERT INTO message(id,session_id,time_created,time_updated,data) VALUES(?1,?2,?3,?4,?5)",
            params![id, session, time_updated, time_updated, data],
        )
        .unwrap();
        db.execute(
            "INSERT INTO part(id,message_id,data) VALUES(?1,?2,'{\"type\":\"text\",\"text\":\"SECRET COMPLETION\"}')",
            params![format!("prt_{id}"), id],
        )
        .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn assistant_messages_yield_cost_model_and_cache_buckets() {
        let row = MessageRow {
            id: "msg_1".into(),
            session_id: "ses_1".into(),
            time_created: 1_778_842_278_253,
            time_updated: 1_778_842_293_500,
            data: assistant_data(
                "kimi-k2.6",
                "opencode-go",
                25036,
                230,
                257,
                40,
                8,
                0.0257322,
                1_778_842_278_253,
            ),
            parent_session_id: Some("ses_parent".into()),
            directory: Some("/Users/x/voicey".into()),
        };
        let record = parse_message(&row).unwrap();
        assert_eq!(record.native_record_id, "msg_1");
        assert_eq!(record.native_session_id.as_deref(), Some("ses_1"));
        assert_eq!(
            record.parent_native_session_id.as_deref(),
            Some("ses_parent")
        );
        assert_eq!(record.model.as_deref(), Some("kimi-k2.6"));
        assert_eq!(record.provider, "opencode-go");
        assert_eq!(record.reported_cost_microusd, Some(25_732));
        assert_eq!(record.usage.total_input_tokens, Some(25036));
        assert_eq!(record.usage.uncached_input_tokens, Some(25036 - 40 - 8));
        assert_eq!(record.usage.cache_read_tokens, Some(40));
        assert_eq!(record.usage.cache_write_tokens, Some(8));
        assert_eq!(record.usage.output_tokens, Some(230));
        assert_eq!(
            record.usage.reasoning_tokens,
            Some(230),
            "clamped to output"
        );
        assert_eq!(record.usage.provider_reported_total_tokens, Some(25523));
        assert_eq!(record.usage.exact_total(), Some(25523));
        assert_eq!(record.occurred_at, "2026-05-15T10:51:18.253+00:00");
    }

    #[test]
    fn user_messages_and_tokenless_assistants_are_skipped() {
        let user = MessageRow {
            id: "m".into(),
            session_id: "s".into(),
            time_created: 1,
            time_updated: 1,
            data: r#"{"role":"user","time":{"created":1}}"#.into(),
            parent_session_id: None,
            directory: None,
        };
        assert!(parse_message(&user).is_none());
        let pending = MessageRow {
            data: r#"{"role":"assistant","modelID":"x","providerID":"y","time":{"created":1}}"#
                .into(),
            ..user
        };
        assert!(parse_message(&pending).is_none());
    }

    #[test]
    fn v2_messages_import_once_in_cache_inclusive_terms() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.db");
        let db = create_db(&path);
        add_v2_tables(&db);
        db.execute(
            "INSERT INTO session_v2(id,project_id,parent_id,slug,directory,version,time_created,time_updated) VALUES('ses_kid','p','ses_root','s','/work/repo','2.1.1',1,1)",
            [],
        )
        .unwrap();
        let legacy = assistant_data("kimi", "opencode-go", 10, 5, 0, 0, 0, 0.1, 1_000);
        insert_message(&db, "msg_old", "ses_old", 1_000, &legacy);
        // Migrated from 1.x under the same id: already read from `message`.
        insert_v2_message(&db, "msg_old", "ses_old", "assistant", 1_000, "{}");
        insert_v2_message(&db, "msg_user", "ses_kid", "user", 2_000, r#"{"text":"SECRET PROMPT"}"#);
        insert_v2_message(
            &db,
            "msg_new",
            "ses_kid",
            "assistant",
            3_000,
            r#"{"time":{"created":3000},"agent":"build","model":{"id":"minimax-m3","providerID":"opencode-go"},"content":[{"type":"text","text":"SECRET"}],"cost":0.002,"tokens":{"input":133,"output":20,"reasoning":46,"cache":{"read":20534,"write":0}}}"#,
        );
        drop(db);
        let history = open_read_only(&path).unwrap();
        let (rows, v2_rows) = read_rows(&history, &OpenCodeCursor::default(), 100).unwrap();
        assert_eq!(rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(), ["msg_old"]);
        assert_eq!(v2_rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(), ["msg_new"]);
        let record = parse_v2_message(&v2_rows[0]).unwrap();
        assert_eq!(record.model.as_deref(), Some("minimax-m3"));
        assert_eq!(record.provider, "opencode-go");
        assert_eq!(record.parent_native_session_id.as_deref(), Some("ses_root"));
        assert_eq!(record.project_path.as_deref(), Some("/work/repo"));
        assert_eq!(record.usage.total_input_tokens, Some(133 + 20534));
        assert_eq!(record.usage.uncached_input_tokens, Some(133));
        assert_eq!(record.usage.output_tokens, Some(66));
        assert_eq!(record.usage.reasoning_tokens, Some(46));
        assert_eq!(record.usage.exact_total(), Some(133 + 20534 + 20 + 46));
        assert_eq!(record.reported_cost_microusd, Some(2_000));

        let after = OpenCodeCursor {
            time_updated: 1_000,
            id: "msg_old".into(),
            v2_time_updated: 3_000,
            v2_id: "msg_new".into(),
        };
        let (rows, v2_rows) = read_rows(&history, &after, 100).unwrap();
        assert!(rows.is_empty() && v2_rows.is_empty(), "both high-water marks hold");
        let (rows, v2_rows) = read_rows(&history, &OpenCodeCursor::default(), 1).unwrap();
        assert_eq!((rows.len(), v2_rows.len()), (1, 0), "1.x rows spend the budget first");
    }

    #[test]
    fn cursors_saved_before_v2_support_still_parse() {
        let cursor: OpenCodeCursor =
            serde_json::from_str(r#"{"timeUpdated":5,"id":"msg_5"}"#).unwrap();
        assert_eq!(cursor.time_updated, 5);
        assert_eq!(cursor.v2_time_updated, 0);
        assert!(cursor.v2_id.is_empty());
    }

    #[test]
    fn read_only_open_refuses_to_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.db");
        create_db(&path);
        let db = open_read_only(&path).unwrap();
        assert!(db
            .execute("INSERT INTO session(id,project_id,slug,directory,title,version,time_created,time_updated) VALUES('a','p','s','d','t','v',1,1)", [])
            .is_err());
    }
}

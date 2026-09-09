//! OpenCode: `~/.local/share/opencode/opencode.db`.
//!
//! Assistant messages store their metadata as JSON in `message.data`:
//! `tokens`, `cost`, `modelID`, `providerID`, and timestamps. Message text
//! lives in a separate `part` table that this importer never touches. The
//! database is opened read-only; the cursor is the `(time_updated, id)`
//! high-water mark, so an edited message is seen again by its id and
//! upserted, replacing whatever was imported from it earlier (e.g. a
//! preliminary row seen before the assistant turn finished).

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
    let rows = {
        let history = open_read_only(&source.location)?;
        let mut statement = history.prepare(
            "SELECT m.id, m.session_id, m.time_created, m.time_updated, m.data, s.parent_id, s.directory
             FROM message m LEFT JOIN session s ON s.id = m.session_id
             WHERE (m.time_updated > ?1 OR (m.time_updated = ?1 AND m.id > ?2))
               AND m.data LIKE '%\"role\":\"assistant\"%'
             ORDER BY m.time_updated, m.id
             LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![cursor.time_updated, cursor.id, max_records as i64],
            |row| {
                Ok(MessageRow {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    time_created: row.get(2)?,
                    time_updated: row.get(3)?,
                    data: row.get(4)?,
                    parent_session_id: row.get(5)?,
                    directory: row.get(6)?,
                })
            },
        )?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let truncated = rows.len() >= max_records;
    let mut skipped = 0_usize;
    let mut records = Vec::with_capacity(rows.len());
    for row in rows {
        cursor = OpenCodeCursor {
            time_updated: row.time_updated,
            id: row.id.clone(),
        };
        match parse_message(&row) {
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

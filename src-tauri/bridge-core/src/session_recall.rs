//! Zero-LLM recall over one session's forest.
//!
//! Search is keyed by `session_id` only. Direct chats with a NULL workspace
//! still do not share a bucket: each chat has its own id. There is no
//! workspace-wide or "account" MATCH from this module.

use crate::BridgeError;
use bridge_protocol::messages::{
    SearchSessionEntriesResult, SessionRecallHit, DEFAULT_RECALL_HIT_LIMIT, MAX_RECALL_HIT_LIMIT,
};
use rusqlite::{params, Connection, Transaction};

/// Kinds whose `text` / `title` / `summary` are conversational enough to index.
/// Raw provider events stay inspectable in the forest but are not recall hits.
const INDEXABLE_KINDS: &str =
    "'user.message','assistant.message','worker.result','compaction','checkpoint','branch.summary'";

pub(crate) fn install_fts(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(&format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS session_entry_fts USING fts5(
            entry_id UNINDEXED,
            session_id UNINDEXED,
            kind UNINDEXED,
            body,
            tokenize = 'unicode61 remove_diacritics 2'
        );
        DROP TRIGGER IF EXISTS session_entries_ai_fts;
        DROP TRIGGER IF EXISTS session_entries_ad_fts;
        DROP TRIGGER IF EXISTS session_entries_au_fts;
        CREATE TRIGGER session_entries_ai_fts AFTER INSERT ON session_entries
        WHEN NEW.kind IN ({INDEXABLE_KINDS})
         AND NEW.context_visibility IN ('eligible','visible')
        BEGIN
          INSERT INTO session_entry_fts(entry_id, session_id, kind, body)
          SELECT NEW.id, NEW.session_id, NEW.kind, {body_sql}
          WHERE length({body_sql}) > 0;
        END;
        CREATE TRIGGER session_entries_ad_fts AFTER DELETE ON session_entries
        BEGIN
          DELETE FROM session_entry_fts WHERE entry_id = OLD.id;
        END;
        CREATE TRIGGER session_entries_au_fts AFTER UPDATE OF payload, kind, session_id, context_visibility
        ON session_entries
        BEGIN
          DELETE FROM session_entry_fts WHERE entry_id = OLD.id;
          INSERT INTO session_entry_fts(entry_id, session_id, kind, body)
          SELECT NEW.id, NEW.session_id, NEW.kind, {new_body_sql}
          WHERE NEW.kind IN ({INDEXABLE_KINDS})
            AND NEW.context_visibility IN ('eligible','visible')
            AND length({new_body_sql}) > 0;
        END;
        INSERT INTO session_entry_fts(entry_id, session_id, kind, body)
        SELECT id, session_id, kind, {table_body_sql}
        FROM session_entries
        WHERE kind IN ({INDEXABLE_KINDS})
          AND context_visibility IN ('eligible','visible')
          AND length({table_body_sql}) > 0
          AND id NOT IN (SELECT entry_id FROM session_entry_fts);",
        body_sql = searchable_body_sql("NEW.payload"),
        new_body_sql = searchable_body_sql("NEW.payload"),
        table_body_sql = searchable_body_sql("payload"),
    ))?;
    Ok(())
}

fn searchable_body_sql(payload_expr: &str) -> String {
    format!(
        "trim(coalesce(json_extract({payload_expr}, '$.text'), '') || ' ' || \
         coalesce(json_extract({payload_expr}, '$.title'), '') || ' ' || \
         coalesce(json_extract({payload_expr}, '$.summary'), ''))"
    )
}

pub fn search(
    db: &Connection,
    session_id: &str,
    query: &str,
    limit: Option<u32>,
) -> Result<SearchSessionEntriesResult, BridgeError> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Err(BridgeError::Invalid(
            "Recall needs a session id; search cannot run across a workspace".into(),
        ));
    }
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",
        params![session_id],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(BridgeError::Invalid("Chat session does not exist".into()));
    }
    let limit = resolve_limit(limit)?;
    let match_query = fts_match_query(query)?;
    let mut statement = db.prepare(
        "SELECT
            session_entry_fts.entry_id,
            session_entry_fts.kind,
            e.sequence,
            snippet(session_entry_fts, 3, '', '', '…', 32),
            session_entry_fts.body,
            e.created_at
         FROM session_entry_fts
         INNER JOIN session_entries AS e ON e.id = session_entry_fts.entry_id
         WHERE session_entry_fts.session_id = ?1
           AND session_entry_fts MATCH ?2
         ORDER BY rank, e.sequence
         LIMIT ?3",
    )?;
    let hits = statement
        .query_map(params![session_id, match_query, limit], |row| {
            let snippet: String = row.get::<_, Option<String>>(3)?.unwrap_or_default();
            let body: String = row.get::<_, Option<String>>(4)?.unwrap_or_default();
            let snippet = if snippet.trim().is_empty() {
                truncate_snippet(&body)
            } else {
                snippet
            };
            Ok(SessionRecallHit {
                entry_id: row.get(0)?,
                kind: row.get(1)?,
                sequence: row.get(2)?,
                snippet,
                created_at: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(SearchSessionEntriesResult {
        session_id: session_id.to_owned(),
        query: query.trim().to_owned(),
        hits,
    })
}

fn resolve_limit(limit: Option<u32>) -> Result<i64, BridgeError> {
    match limit {
        None => Ok(DEFAULT_RECALL_HIT_LIMIT as i64),
        Some(value) if (1..=MAX_RECALL_HIT_LIMIT).contains(&value) => Ok(value as i64),
        Some(_) => Err(BridgeError::Invalid(format!(
            "Recall limit must be between 1 and {MAX_RECALL_HIT_LIMIT}"
        ))),
    }
}

/// Phrase-wrap alphanumeric tokens so user input cannot be FTS syntax.
pub fn fts_match_query(raw: &str) -> Result<String, BridgeError> {
    let tokens: Vec<String> = raw
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .filter(|token| {
            !matches!(
                token.to_ascii_uppercase().as_str(),
                "AND" | "OR" | "NOT" | "NEAR"
            )
        })
        .map(str::to_string)
        .collect();
    if tokens.is_empty() {
        return Err(BridgeError::Invalid(
            "Recall needs a word to search for in this chat".into(),
        ));
    }
    Ok(tokens
        .into_iter()
        .map(|token| format!("\"{token}\""))
        .collect::<Vec<_>>()
        .join(" AND "))
}

pub fn format_reply(result: &SearchSessionEntriesResult) -> String {
    if result.hits.is_empty() {
        return format!(
            "No matches in this chat for \"{}\". Search only looks at this session.",
            result.query
        );
    }
    let mut out = format!("Recall in this chat ({}):\n", result.hits.len());
    for (index, hit) in result.hits.iter().enumerate() {
        let snippet = hit.snippet.replace('\n', " ");
        out.push_str(&format!(
            "\n{}. {} · #{}\n   {}\n",
            index + 1,
            hit.kind,
            hit.sequence,
            snippet
        ));
    }
    out
}

fn truncate_snippet(body: &str) -> String {
    let trimmed = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if trimmed.chars().count() <= 160 {
        trimmed
    } else {
        format!("{}…", trimmed.chars().take(160).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{self, append_session_entry};
    use rusqlite::params;
    use serde_json::json;

    fn recall_db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db = store::open(&dir.path().join("bridge.db")).unwrap();
        (dir, db)
    }

    fn insert_chat(db: &Connection, id: &str, ended: bool) {
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind)
             VALUES(?1,NULL,'codex','Chat','idle','estimated','direct')",
            params![id],
        )
        .unwrap();
        if ended {
            db.execute(
                "UPDATE sessions SET status='stopped', ended_at='2020-01-01T00:00:00Z' WHERE id=?1",
                params![id],
            )
            .unwrap();
        }
    }

    fn add_message(db: &Connection, session_id: &str, kind: &str, payload: serde_json::Value) {
        append_session_entry(db, session_id, None, kind, &payload, None, "eligible", None).unwrap();
    }

    #[test]
    fn search_in_session_a_does_not_return_session_b() {
        let (_dir, db) = recall_db();
        insert_chat(&db, "a", false);
        insert_chat(&db, "b", false);
        add_message(
            &db,
            "a",
            "user.message",
            json!({"text": "alpha-token-only-in-a"}),
        );
        add_message(
            &db,
            "b",
            "user.message",
            json!({"text": "beta-token-only-in-b"}),
        );

        let from_a = search(&db, "a", "alpha-token-only-in-a", None).unwrap();
        assert_eq!(from_a.hits.len(), 1);
        assert!(from_a.hits[0].snippet.contains("alpha-token-only-in-a"));
        assert!(search(&db, "a", "beta-token-only-in-b", None)
            .unwrap()
            .hits
            .is_empty());

        let from_b = search(&db, "b", "beta-token-only-in-b", None).unwrap();
        assert_eq!(from_b.hits.len(), 1);
        assert!(search(&db, "b", "alpha-token-only-in-a", None)
            .unwrap()
            .hits
            .is_empty());
    }

    #[test]
    fn two_direct_chats_with_null_workspace_do_not_share_recall() {
        let (_dir, db) = recall_db();
        insert_chat(&db, "direct-a", false);
        insert_chat(&db, "direct-b", false);
        add_message(
            &db,
            "direct-a",
            "assistant.message",
            json!({"text": "shared-sounding cookie recipe for a"}),
        );
        add_message(
            &db,
            "direct-b",
            "assistant.message",
            json!({"text": "shared-sounding cookie recipe for b"}),
        );
        let hits = search(&db, "direct-a", "cookie recipe", None).unwrap();
        assert_eq!(hits.hits.len(), 1);
        assert!(hits.hits[0].snippet.contains("for a"));
        assert!(!hits.hits[0].snippet.contains("for b"));
    }

    #[test]
    fn empty_session_id_is_rejected() {
        let (_dir, db) = recall_db();
        let error = search(&db, "  ", "anything", None).unwrap_err();
        assert!(error.to_string().contains("session id"));
    }

    #[test]
    fn fts_operators_cannot_unscope_the_query() {
        let (_dir, db) = recall_db();
        insert_chat(&db, "a", false);
        insert_chat(&db, "b", false);
        add_message(&db, "a", "user.message", json!({"text": "hello from a"}));
        add_message(
            &db,
            "b",
            "user.message",
            json!({"text": "secret-unique-in-b"}),
        );
        let hits = search(&db, "a", "\" OR secret-unique-in-b", None).unwrap();
        assert!(
            hits.hits.is_empty(),
            "operator soup must not leak the other session: {hits:?}"
        );
        assert!(fts_match_query("*").is_err());
    }

    #[test]
    fn ended_entries_in_this_session_stay_searchable() {
        let (_dir, db) = recall_db();
        insert_chat(&db, "old", true);
        add_message(
            &db,
            "old",
            "user.message",
            json!({"text": "decision we made last year"}),
        );
        let hits = search(&db, "old", "decision last year", None).unwrap();
        assert_eq!(hits.hits.len(), 1);
    }

    #[test]
    fn finds_user_and_assistant_text() {
        let (_dir, db) = recall_db();
        insert_chat(&db, "s", false);
        add_message(
            &db,
            "s",
            "user.message",
            json!({"text": "ship the fts index"}),
        );
        add_message(
            &db,
            "s",
            "assistant.message",
            json!({"text": "the fts index is live"}),
        );
        add_message(
            &db,
            "s",
            "provider.unknown",
            json!({"raw": "fts index must not appear"}),
        );
        let hits = search(&db, "s", "fts index", None).unwrap();
        assert_eq!(hits.hits.len(), 2);
        let kinds: Vec<_> = hits.hits.iter().map(|hit| hit.kind.as_str()).collect();
        assert!(kinds.contains(&"user.message"));
        assert!(kinds.contains(&"assistant.message"));
        assert!(!kinds.contains(&"provider.unknown"));
    }

    #[test]
    fn missing_session_is_invalid_not_an_unscoped_scan() {
        let (_dir, db) = recall_db();
        let error = search(&db, "no-such-session", "hello", None).unwrap_err();
        assert!(error.to_string().contains("does not exist"));
    }

    #[test]
    fn out_of_range_limit_is_rejected() {
        let (_dir, db) = recall_db();
        insert_chat(&db, "s", false);
        assert!(search(&db, "s", "hello", Some(0)).is_err());
        assert!(search(&db, "s", "hello", Some(51)).is_err());
    }
}

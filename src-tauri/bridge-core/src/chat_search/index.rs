//! The one row per chat that says what the chat is about.
//!
//! Users remember a chat's topic, and the topic is stated in three places: its
//! title, its first message, and its latest summary. `chat_digests` holds those
//! three, truncated, and `chat_digest_fts` indexes them, so a vague "the plugins
//! one" can match a chat whose every entry says something more specific.
//!
//! Triggers keep a digest current: a new or renamed chat, its first user
//! message, and every compaction, checkpoint or branch summary rewrite it. The
//! FTS table is external-content over `chat_digests`, keyed by that table's
//! integer primary key, which `VACUUM` never renumbers.
//!
//! Two fts5vocab tables expose document frequencies, which is how the parser
//! tells a rare word from a common one without a second index.

use rusqlite::Transaction;

use crate::BridgeError;

/// Characters of the first user message a digest keeps.
pub const FIRST_MESSAGE_CHARS: i64 = 500;
/// Characters of the latest summary a digest keeps. With the message and a
/// title this keeps a digest under 2 KB.
pub const SUMMARY_CHARS: i64 = 1_000;

const SUMMARY_KINDS: &str = "'compaction','checkpoint','branch.summary'";

/// Session kinds nobody sees in a list, as a SQL list. Mirrors
/// `work_briefing_config::is_hidden_session_kind`; a test holds them equal.
/// The digest triggers bake this list in when migration 63 runs, so a kind
/// added later is still filtered at query time but may keep stale digests.
pub const HIDDEN_KINDS_SQL: &str =
    "'briefing','suggestion','extraction','outcome_evaluation','consolidation','chat_search','connector'";

/// Upsert the digest for the session whose id is the SQL expression `id`.
fn rebuild_sql(id: &str) -> String {
    format!(
        "INSERT INTO chat_digests(session_id, body)
         SELECT s.id, trim(
             coalesce(s.title, '') || ' ' ||
             CASE WHEN s.label IS NOT NULL AND s.label IS NOT s.title AND s.label <> 'Chat'
                  THEN s.label ELSE '' END || ' ' ||
             coalesce(substr((
                 SELECT json_extract(e.payload, '$.text') FROM session_entries e
                 WHERE e.session_id = s.id AND e.kind = 'user.message'
                   AND e.context_visibility IN ('eligible','visible')
                 ORDER BY e.sequence LIMIT 1), 1, {FIRST_MESSAGE_CHARS}), '') || ' ' ||
             coalesce(substr((
                 SELECT coalesce(json_extract(e.payload, '$.summary'), json_extract(e.payload, '$.text'))
                 FROM session_entries e
                 WHERE e.session_id = s.id AND e.kind IN ({SUMMARY_KINDS})
                 ORDER BY e.sequence DESC LIMIT 1), 1, {SUMMARY_CHARS}), ''))
         FROM sessions s
         WHERE s.id = {id} AND s.parent_session_id IS NULL
           AND coalesce(s.kind, 'direct') NOT IN ({HIDDEN_KINDS_SQL})
         ON CONFLICT(session_id) DO UPDATE SET body = excluded.body
         WHERE chat_digests.body IS NOT excluded.body;"
    )
}

pub(crate) fn install(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS chat_digests (
            id INTEGER PRIMARY KEY,
            session_id TEXT NOT NULL UNIQUE,
            body TEXT NOT NULL
        );
        CREATE VIRTUAL TABLE IF NOT EXISTS chat_digest_fts USING fts5(
            body,
            content = 'chat_digests',
            content_rowid = 'id',
            tokenize = 'unicode61 remove_diacritics 2'
        );
        CREATE VIRTUAL TABLE IF NOT EXISTS session_entry_vocab
            USING fts5vocab(session_entry_fts, 'row');
        CREATE VIRTUAL TABLE IF NOT EXISTS chat_digest_vocab
            USING fts5vocab(chat_digest_fts, 'row');

        CREATE TRIGGER IF NOT EXISTS chat_digests_ai AFTER INSERT ON chat_digests BEGIN
          INSERT INTO chat_digest_fts(rowid, body) VALUES (NEW.id, NEW.body);
        END;
        CREATE TRIGGER IF NOT EXISTS chat_digests_ad AFTER DELETE ON chat_digests BEGIN
          INSERT INTO chat_digest_fts(chat_digest_fts, rowid, body) VALUES ('delete', OLD.id, OLD.body);
        END;
        CREATE TRIGGER IF NOT EXISTS chat_digests_au AFTER UPDATE ON chat_digests BEGIN
          INSERT INTO chat_digest_fts(chat_digest_fts, rowid, body) VALUES ('delete', OLD.id, OLD.body);
          INSERT INTO chat_digest_fts(rowid, body) VALUES (NEW.id, NEW.body);
        END;

        CREATE TRIGGER IF NOT EXISTS chat_digest_sessions_ai AFTER INSERT ON sessions BEGIN
          {on_session}
        END;
        CREATE TRIGGER IF NOT EXISTS chat_digest_sessions_au
        AFTER UPDATE OF title, label, kind, parent_session_id ON sessions BEGIN
          DELETE FROM chat_digests WHERE session_id = NEW.id
            AND (NEW.parent_session_id IS NOT NULL
                 OR coalesce(NEW.kind, 'direct') IN ({HIDDEN_KINDS_SQL}));
          {on_session}
        END;
        CREATE TRIGGER IF NOT EXISTS chat_digest_sessions_ad AFTER DELETE ON sessions BEGIN
          DELETE FROM chat_digests WHERE session_id = OLD.id;
        END;

        CREATE TRIGGER IF NOT EXISTS chat_digest_entries_ai AFTER INSERT ON session_entries
        WHEN NEW.kind IN ({SUMMARY_KINDS})
          OR (NEW.kind = 'user.message' AND NOT EXISTS (
                SELECT 1 FROM session_entries e
                WHERE e.session_id = NEW.session_id AND e.kind = 'user.message' AND e.id <> NEW.id))
        BEGIN
          {on_entry_new}
        END;
        CREATE TRIGGER IF NOT EXISTS chat_digest_entries_au
        AFTER UPDATE OF payload, kind, context_visibility ON session_entries
        WHEN NEW.kind IN ('user.message', {SUMMARY_KINDS}) OR OLD.kind IN ('user.message', {SUMMARY_KINDS})
        BEGIN
          {on_entry_new}
        END;
        CREATE TRIGGER IF NOT EXISTS chat_digest_entries_ad AFTER DELETE ON session_entries
        WHEN OLD.kind IN ('user.message', {SUMMARY_KINDS})
        BEGIN
          {on_entry_old}
        END;

        {backfill}",
        on_session = rebuild_sql("NEW.id"),
        on_entry_new = rebuild_sql("NEW.session_id"),
        on_entry_old = rebuild_sql("OLD.session_id"),
        backfill = rebuild_sql("s.id").replace("WHERE s.id = s.id AND", "WHERE"),
    ))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{self, append_session_entry};
    use rusqlite::{params, Connection};
    use serde_json::json;

    fn db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db = store::open(&dir.path().join("bridge.db")).unwrap();
        (dir, db)
    }

    fn digest(db: &Connection, id: &str) -> Option<String> {
        db.query_row("SELECT body FROM chat_digests WHERE session_id=?1", params![id], |row| row.get(0))
            .ok()
    }

    fn matches(db: &Connection, word: &str) -> Vec<String> {
        let mut statement = db
            .prepare(
                "SELECT d.session_id FROM chat_digest_fts f JOIN chat_digests d ON d.id = f.rowid
                 WHERE chat_digest_fts MATCH ?1 ORDER BY d.session_id",
            )
            .unwrap();
        statement
            .query_map(params![format!("\"{word}\"")], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    fn chat(db: &Connection, id: &str, title: Option<&str>) {
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind,title)
             VALUES(?1,NULL,'codex','Chat','idle','estimated','direct',?2)",
            params![id, title],
        )
        .unwrap();
    }

    #[test]
    fn digest_tracks_title_first_message_and_latest_summary() {
        let (_dir, db) = db();
        chat(&db, "s", Some("Plugins catalog"));
        assert_eq!(digest(&db, "s").as_deref(), Some("Plugins catalog"));

        append_session_entry(&db, "s", None, "user.message", &json!({"text":"why does the marketplace hang"}), None, "eligible", None).unwrap();
        append_session_entry(&db, "s", None, "user.message", &json!({"text":"second message never digested"}), None, "eligible", None).unwrap();
        let body = digest(&db, "s").unwrap();
        assert!(body.contains("marketplace hang"), "{body}");
        assert!(!body.contains("second message"), "{body}");

        append_session_entry(&db, "s", None, "compaction", &json!({"summary":"bounded the CLI with a timeout"}), None, "eligible", None).unwrap();
        append_session_entry(&db, "s", None, "checkpoint", &json!({"summary":"latest checkpoint wins"}), None, "eligible", None).unwrap();
        let body = digest(&db, "s").unwrap();
        assert!(body.contains("latest checkpoint wins"), "{body}");
        assert!(!body.contains("bounded the CLI"), "only the latest summary: {body}");

        db.execute("UPDATE sessions SET title='Renamed topic' WHERE id='s'", []).unwrap();
        assert!(digest(&db, "s").unwrap().starts_with("Renamed topic"));
        assert_eq!(matches(&db, "renamed"), vec!["s"]);
        assert!(matches(&db, "plugins").is_empty(), "the FTS index follows the rename");

        db.execute("DELETE FROM session_heads WHERE session_id='s'", []).unwrap();
        db.execute("DELETE FROM session_entries WHERE session_id='s'", []).unwrap();
        db.execute("DELETE FROM sessions WHERE id='s'", []).unwrap();
        assert_eq!(digest(&db, "s"), None);
        assert!(matches(&db, "renamed").is_empty());
    }

    #[test]
    fn workers_and_hidden_sessions_get_no_digest() {
        let (_dir, db) = db();
        chat(&db, "parent", Some("Parent"));
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind,title,parent_session_id,depth)
             VALUES('worker',NULL,'codex','Worker','idle','estimated','direct','Worker topic','parent',1)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind,title)
             VALUES('hidden',NULL,'claude','Search','idle','reported','chat_search','Hidden topic')",
            [],
        )
        .unwrap();
        assert_eq!(digest(&db, "worker"), None);
        assert_eq!(digest(&db, "hidden"), None);
        assert!(digest(&db, "parent").is_some());
    }

    #[test]
    fn hidden_kind_list_matches_the_core_predicate() {
        for kind in HIDDEN_KINDS_SQL.split(',') {
            let kind = kind.trim_matches('\'');
            assert!(
                crate::work_briefing_config::is_hidden_session_kind(Some(kind)),
                "{kind} is listed hidden here but not in core"
            );
        }
        for kind in [
            crate::work_briefing_config::BRIEFING_SESSION_KIND,
            crate::suggestion_engine::SUGGESTION_SESSION_KIND,
            crate::memory_extraction::EXTRACTION_SESSION_KIND,
            crate::routing_evaluation::EVALUATION_SESSION_KIND,
            crate::memory_consolidation::CONSOLIDATION_SESSION_KIND,
            super::super::CHAT_SEARCH_SESSION_KIND,
            crate::connector_runs_live::CONNECTOR_SESSION_KIND,
        ] {
            assert!(HIDDEN_KINDS_SQL.contains(&format!("'{kind}'")), "{kind} missing from the SQL list");
        }
    }
}

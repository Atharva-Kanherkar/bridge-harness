//! One session's durable record, written out as newline-delimited JSON.
//!
//! The session forest is already the authoritative history — this module adds
//! no storage and no second read path. It exists because a record you cannot
//! get out of the app is a record you cannot check: the export is what lets a
//! reader diff two runs, grep for the tool call that failed, or hand a
//! transcript to something that is not Bridge.
//!
//! Three rules shape the format:
//!
//! - **Every line stands alone.** A payload containing newlines is escaped by
//!   `serde_json`, never emitted raw, so `head`, `tail`, `grep` and `jq -c`
//!   all work on a partially written or truncated file.
//! - **The file describes itself.** The first line names the schema version and
//!   the session it came from; the last line carries per-kind counts and a
//!   digest over the entry lines, so a truncated export is detectable rather
//!   than merely short.
//! - **It exports what is stored, unchanged.** Payloads are copied verbatim.
//!   Redaction happened before the turn left the machine (see
//!   `secret_interception`), and re-deciding it here would mean the export and
//!   the transcript disagree about what the session contained.

use crate::{model::SessionEntry, session_forest::SessionForest, store, BridgeError};
use bridge_protocol::messages::JsSafeU64;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Bumped when the shape of a line changes. Readers should refuse a major they
/// do not know rather than guess at a field.
pub const EXPORT_SCHEMA_VERSION: u64 = 1;

/// The wire contract is the only definition of these two: a second core copy
/// would be one more thing to keep in lockstep, and this method has no shape
/// of its own to defend.
pub use bridge_protocol::messages::{
    ExportSessionTranscriptResult as TranscriptExport, TranscriptExportScope as ExportScope,
};

fn scope_as_str(scope: ExportScope) -> &'static str {
    match scope {
        ExportScope::ActiveBranch => "active_branch",
        ExportScope::Forest => "forest",
    }
}

/// The session identity written into the header line.
struct SessionHeader {
    workspace_id: Option<String>,
    harness: String,
    label: String,
    status: String,
    model: Option<String>,
    effort: Option<String>,
    started_at: Option<String>,
    ended_at: Option<String>,
}

pub fn export(
    db: &Connection,
    data_dir: &Path,
    session_id: &str,
    scope: ExportScope,
    include_hidden: bool,
    destination: Option<&str>,
) -> Result<TranscriptExport, BridgeError> {
    let header = read_session_header(db, session_id)?;
    let entries = collect_entries(db, session_id, scope, include_hidden)?;
    let exported_at = chrono::Utc::now();
    let path = resolve_destination(data_dir, session_id, destination, &exported_at)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            BridgeError::Invalid(format!(
                "cannot create the export directory {}: {error}",
                parent.display()
            ))
        })?;
    }

    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut digest = Sha256::new();
    let mut lines: Vec<String> = Vec::with_capacity(entries.len() + 2);

    lines.push(line(&json!({
        "type": "header",
        "schemaVersion": EXPORT_SCHEMA_VERSION,
        "exportedAt": exported_at.to_rfc3339(),
        "scope": scope_as_str(scope),
        "includeHidden": include_hidden,
        "session": {
            "id": session_id,
            "workspaceId": header.workspace_id,
            "harness": header.harness,
            "label": header.label,
            "status": header.status,
            "model": header.model,
            "effort": header.effort,
            "startedAt": header.started_at,
            "endedAt": header.ended_at,
        },
    }))?);

    // Turn index is derived, not stored. It is written out so every consumer
    // agrees on the answer rather than each deriving one — and so an export
    // that omits the hidden boundary entries still says which turn a row was
    // in, which a reader could no longer work out from the file alone.
    for entry in &entries {
        *counts.entry(entry.kind.clone()).or_default() += 1;
        let rendered = line(&entry_record(entry))?;
        digest.update(rendered.as_bytes());
        lines.push(rendered);
    }

    lines.push(line(&json!({
        "type": "footer",
        "entryCount": entries.len() as u64,
        "counts": counts,
        "digest": format!("sha256:{:x}", digest.finalize()),
    }))?);

    let digest_line: Value = serde_json::from_str(lines.last().expect("footer was pushed"))
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let digest_value = digest_line["digest"].as_str().unwrap_or_default().to_owned();

    let body = lines.join("\n") + "\n";
    let mut file = std::fs::File::create(&path).map_err(|error| {
        BridgeError::Invalid(format!("cannot write the export {}: {error}", path.display()))
    })?;
    file.write_all(body.as_bytes())
        .and_then(|()| file.flush())
        .map_err(|error| {
            BridgeError::Invalid(format!("cannot write the export {}: {error}", path.display()))
        })?;

    Ok(TranscriptExport {
        session_id: session_id.to_owned(),
        path: path.to_string_lossy().into_owned(),
        scope,
        schema_version: js_safe(EXPORT_SCHEMA_VERSION)?,
        line_count: js_safe(lines.len() as u64)?,
        entry_count: js_safe(entries.len() as u64)?,
        bytes: js_safe(body.len() as u64)?,
        digest: digest_value,
        exported_at: exported_at.to_rfc3339(),
    })
}

/// One entry, flattened so the fields a reader greps for are top level and the
/// stored document is still present whole under `payload`.
fn entry_record(entry: &ExportEntry) -> Value {
    let payload = &entry.entry.payload;
    let field = |name: &str| payload.get(name).and_then(Value::as_str);
    json!({
        "type": "entry",
        "sequence": entry.entry.sequence,
        "entryId": entry.entry.id,
        "parentEntryId": entry.entry.parent_entry_id,
        "kind": entry.entry.kind,
        "createdAt": entry.entry.created_at,
        "contextVisibility": entry.entry.context_visibility,
        "semanticSchemaVersion": entry.entry.semantic_schema_version,
        "tokenEstimate": entry.entry.token_estimate,
        "providerEventId": entry.entry.provider_event_id,
        "turnIndex": entry.turn_index,
        "onActiveBranch": entry.on_active_branch,
        "role": field("role"),
        "status": field("status"),
        "title": field("title"),
        "text": field("text"),
        "data": payload.get("data").cloned().unwrap_or_else(|| json!({})),
        "providerMeta": payload.get("providerMeta").cloned().unwrap_or_else(|| json!({})),
        "payload": payload,
    })
}

struct ExportEntry {
    entry: SessionEntry,
    on_active_branch: bool,
    /// Which turn this entry fell in, counted over the scoped sequence before
    /// hidden entries are dropped.
    turn_index: u64,
}

impl std::ops::Deref for ExportEntry {
    type Target = SessionEntry;
    fn deref(&self) -> &SessionEntry {
        &self.entry
    }
}

fn collect_entries(
    db: &Connection,
    session_id: &str,
    scope: ExportScope,
    include_hidden: bool,
) -> Result<Vec<ExportEntry>, BridgeError> {
    let forest = SessionForest::new(db);
    let active: std::collections::HashSet<String> = forest
        .active_branch(session_id)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?
        .into_iter()
        .map(|entry| entry.id)
        .collect();
    let mut entries = store::session_entries(db, session_id)?
        .into_iter()
        .map(|entry| {
            let on_active_branch = active.contains(&entry.id);
            ExportEntry {
                entry,
                on_active_branch,
                turn_index: 0,
            }
        })
        .filter(|entry| scope == ExportScope::Forest || entry.on_active_branch)
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.sequence);

    // Number the turns *before* dropping the hidden entries, because the
    // boundaries that define a turn are themselves hidden. Deriving after the
    // filter labelled every row of an `includeHidden: false` export turn zero,
    // which is worse than omitting the field: it reads like a real answer.
    //
    // The boundary carries its own ordinal (stamped when it was recorded), so
    // the number here is the session's turn and not this export's count of
    // them. Entries written before the stamp existed fall back to counting.
    let mut turn_index = 0;
    for entry in &mut entries {
        if entry.kind == "turn.started" {
            turn_index = entry
                .payload
                .get("data")
                .and_then(|data| data.get("turnIndex"))
                .and_then(Value::as_u64)
                .unwrap_or(turn_index + 1);
        }
        entry.turn_index = turn_index;
    }

    entries.retain(|entry| include_hidden || entry.context_visibility != "hidden");
    Ok(entries)
}

fn read_session_header(db: &Connection, session_id: &str) -> Result<SessionHeader, BridgeError> {
    db.query_row(
        "SELECT workspace_id,harness,label,status,model,effort,started_at,ended_at
         FROM sessions WHERE id=?1",
        params![session_id],
        |row| {
            Ok(SessionHeader {
                workspace_id: row.get(0)?,
                harness: row.get(1)?,
                label: row.get(2)?,
                status: row.get(3)?,
                model: row.get(4)?,
                effort: row.get(5)?,
                started_at: row.get(6)?,
                ended_at: row.get(7)?,
            })
        },
    )
    .optional()?
    .ok_or_else(|| BridgeError::Invalid(format!("unknown session {session_id}")))
}

fn resolve_destination(
    data_dir: &Path,
    session_id: &str,
    destination: Option<&str>,
    exported_at: &chrono::DateTime<chrono::Utc>,
) -> Result<PathBuf, BridgeError> {
    if let Some(destination) = destination.map(str::trim).filter(|value| !value.is_empty()) {
        let path = PathBuf::from(destination);
        if path.is_relative() {
            return Err(BridgeError::Invalid(
                "destinationPath must be absolute".into(),
            ));
        }
        return Ok(path);
    }
    // Millisecond precision, then a counted suffix. Two exports of the same
    // session a moment apart are a normal thing to want — before and after a
    // retry, say — and silently overwriting the first would destroy the very
    // comparison the second was taken for.
    let stamp = exported_at.format("%Y%m%dT%H%M%S%.3fZ").to_string().replace('.', "");
    let directory = data_dir.join("exports");
    let stem = format!("{}-{stamp}", sanitize(session_id));
    let mut candidate = directory.join(format!("{stem}.jsonl"));
    for attempt in 2..1_000 {
        if !candidate.exists() {
            return Ok(candidate);
        }
        candidate = directory.join(format!("{stem}-{attempt}.jsonl"));
    }
    Err(BridgeError::Invalid(
        "cannot find an unused export filename".into(),
    ))
}

/// A session id is generated, but the export names a file with it, so it is
/// constrained here rather than trusted.
fn sanitize(session_id: &str) -> String {
    session_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn js_safe(value: u64) -> Result<JsSafeU64, BridgeError> {
    JsSafeU64::new(value).map_err(|reason| BridgeError::Invalid(format!("{value}: {reason}")))
}

fn line(value: &Value) -> Result<String, BridgeError> {
    serde_json::to_string(value)
        .map_err(|error| BridgeError::Invalid(format!("cannot serialize an export line: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::NormalizedEvent;

    fn session_db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db = store::open(&dir.path().join("bridge.db")).unwrap();
        db.execute(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/demo','now')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at)
             VALUES('w','p','Kyoto','Task','bridge/task','/tmp/ws','idle','now')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,model,metric_source)
             VALUES('s','w','codex','Chat','ready','gpt-5','reported')",
            [],
        )
        .unwrap();
        (dir, db)
    }

    fn say(db: &Connection, kind: &str, text: &str) {
        let mut event = NormalizedEvent::new(kind);
        event.text = Some(text.into());
        if kind == "message.completed" {
            event.role = Some("assistant".into());
        }
        store::session_event(db, "s", &event, &serde_json::json!({"adapter":"codex"})).unwrap();
    }

    fn read_lines(export: &TranscriptExport) -> Vec<Value> {
        std::fs::read_to_string(&export.path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).expect("every line is standalone JSON"))
            .collect()
    }

    fn export_forest(dir: &tempfile::TempDir, db: &Connection) -> TranscriptExport {
        export(db, dir.path(), "s", ExportScope::Forest, true, None).unwrap()
    }

    #[test]
    fn header_entries_and_footer_frame_one_session() {
        let (dir, db) = session_db();
        say(&db, "turn.started", "");
        say(&db, "reasoning.completed", "I should read the test first.");
        say(&db, "message.completed", "Done.");

        let export = export_forest(&dir, &db);
        let lines = read_lines(&export);

        assert_eq!(lines.len() as u64, export.line_count.get());
        assert_eq!(lines[0]["type"], "header");
        assert_eq!(lines[0]["schemaVersion"], EXPORT_SCHEMA_VERSION);
        assert_eq!(lines[0]["session"]["harness"], "codex");
        assert_eq!(lines[0]["session"]["model"], "gpt-5");

        let kinds = lines[1..lines.len() - 1]
            .iter()
            .map(|line| line["kind"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec!["turn.started", "reasoning.completed", "assistant.message"]
        );

        let footer = lines.last().unwrap();
        assert_eq!(footer["type"], "footer");
        assert_eq!(footer["entryCount"], 3);
        assert_eq!(footer["counts"]["assistant.message"], 1);
        assert!(footer["digest"].as_str().unwrap().starts_with("sha256:"));
        assert_eq!(footer["digest"], export.digest.as_str());
    }

    #[test]
    fn the_record_carries_what_the_transcript_cannot_show_later() {
        // The point of the export: a reasoning trace and a failed tool are in
        // the file with their text intact, not summarized away.
        let (dir, db) = session_db();
        say(&db, "reasoning.completed", "The lock is held by the reader.");
        let mut failed = NormalizedEvent::new("tool.completed");
        failed.item_id = Some("call-1".into());
        failed.status = Some("failed".into());
        failed.title = Some("cargo test".into());
        failed.data = serde_json::json!({"exitCode": 101});
        store::session_event(&db, "s", &failed, &serde_json::json!({})).unwrap();

        let lines = read_lines(&export_forest(&dir, &db));
        let thought = lines
            .iter()
            .find(|line| line["kind"] == "reasoning.completed")
            .unwrap();
        assert_eq!(thought["text"], "The lock is held by the reader.");
        let tool = lines.iter().find(|line| line["kind"] == "tool.completed").unwrap();
        assert_eq!(tool["status"], "failed");
        assert_eq!(tool["data"]["exitCode"], 101);
    }

    #[test]
    fn turn_index_counts_boundaries_and_leaves_the_preamble_at_zero() {
        let (dir, db) = session_db();
        say(&db, "message.completed", "before any boundary");
        say(&db, "turn.started", "");
        say(&db, "message.completed", "inside turn one");
        say(&db, "turn.completed", "");
        say(&db, "turn.started", "");
        say(&db, "message.completed", "inside turn two");

        let lines = read_lines(&export_forest(&dir, &db));
        let indexes = lines
            .iter()
            .filter(|line| line["type"] == "entry")
            .map(|line| line["turnIndex"].as_u64().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(indexes, vec![0, 1, 1, 1, 2, 2]);
    }

    #[test]
    fn a_turn_boundary_carries_the_sessions_turn_number_not_a_readers_count() {
        // The number has to survive being read through a window. A client that
        // loads only the newest page counts from what it loaded; the stamp is
        // what makes its answer and this file's answer the same answer.
        let (dir, db) = session_db();
        for _ in 0..3 {
            say(&db, "turn.started", "");
            say(&db, "message.completed", "reply");
        }
        let stamped = store::session_events_tail(&db, "s", 10)
            .unwrap()
            .into_iter()
            .filter(|event| event.kind == "turn.started")
            .map(|event| event.data["turnIndex"].as_u64().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(stamped, vec![1, 2, 3], "replay must carry the ordinal");

        let lines = read_lines(&export_forest(&dir, &db));
        let exported = lines
            .iter()
            .filter(|line| line["kind"] == "turn.started")
            .map(|line| line["turnIndex"].as_u64().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(exported, stamped);
    }

    #[test]
    fn a_session_recorded_before_the_stamp_still_numbers_its_turns() {
        // Older entries have no ordinal. Counting is the fallback, so an
        // existing session does not export a file full of turn zero.
        let (dir, db) = session_db();
        for index in 0..3 {
            store::append_session_entry(
                &db, "s", None, "turn.started", &json!({"protocolVersion":1,"data":{}}), None, "hidden", None,
            )
            .unwrap();
            store::append_session_entry(
                &db, "s", None, "assistant.message",
                &json!({"protocolVersion":1,"text":format!("reply {index}"),"data":{}}), None, "eligible", None,
            )
            .unwrap();
        }
        let turns = read_lines(&export_forest(&dir, &db))
            .into_iter()
            .filter(|line| line["kind"] == "assistant.message")
            .map(|line| line["turnIndex"].as_u64().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(turns, vec![1, 2, 3]);
    }

    #[test]
    fn omitting_hidden_entries_keeps_the_turn_numbers_of_the_rows_that_remain() {
        // The boundaries that define a turn are themselves hidden, so deriving
        // the index after the filter labelled every row turn zero — which
        // reads like a real answer rather than a missing one.
        let (dir, db) = session_db();
        say(&db, "turn.started", "");
        say(&db, "message.completed", "first turn");
        say(&db, "turn.completed", "");
        say(&db, "turn.started", "");
        say(&db, "message.completed", "second turn");
        say(&db, "turn.completed", "");
        say(&db, "turn.started", "");
        say(&db, "message.completed", "third turn");

        let spoken = export(&db, dir.path(), "s", ExportScope::Forest, false, None).unwrap();
        let rows = read_lines(&spoken)
            .into_iter()
            .filter(|line| line["type"] == "entry")
            .map(|line| (line["text"].as_str().unwrap().to_owned(), line["turnIndex"].as_u64().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(
            rows,
            vec![
                ("first turn".to_owned(), 1),
                ("second turn".to_owned(), 2),
                ("third turn".to_owned(), 3),
            ]
        );

        // And the same rows carry the same numbers when the boundaries are in
        // the file, so the two exports of one session never disagree.
        let everything = export_forest(&dir, &db);
        let with_hidden = read_lines(&everything)
            .into_iter()
            .filter(|line| line["type"] == "entry" && line["kind"] == "assistant.message")
            .map(|line| (line["text"].as_str().unwrap().to_owned(), line["turnIndex"].as_u64().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(with_hidden, rows);
    }

    #[test]
    fn excluding_hidden_drops_only_the_control_entries() {
        let (dir, db) = session_db();
        say(&db, "turn.started", "");
        say(&db, "message.completed", "hello");
        say(&db, "usage.updated", "");

        let everything = export_forest(&dir, &db);
        let spoken = export(&db, dir.path(), "s", ExportScope::Forest, false, None).unwrap();
        assert_eq!(everything.entry_count.get(), 3);
        assert_eq!(spoken.entry_count.get(), 1);
        let kinds = read_lines(&spoken)
            .iter()
            .filter(|line| line["type"] == "entry")
            .map(|line| line["kind"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(kinds, vec!["assistant.message"]);
    }

    #[test]
    fn active_branch_scope_excludes_abandoned_branches() {
        let (dir, db) = session_db();
        say(&db, "message.completed", "first");
        say(&db, "message.completed", "second");
        // Rewind to the first entry and speak again: the second entry is now
        // an abandoned sibling, still history but not the conversation.
        let forest = SessionForest::new(&db);
        let entries = store::session_entries(&db, "s").unwrap();
        forest.move_head("s", Some(&entries[0].id)).unwrap();
        say(&db, "message.completed", "instead");

        let whole = export_forest(&dir, &db);
        let branch = export(&db, dir.path(), "s", ExportScope::ActiveBranch, true, None).unwrap();
        assert_eq!(whole.entry_count.get(), 3);
        assert_eq!(branch.entry_count.get(), 2);

        let texts = read_lines(&branch)
            .iter()
            .filter(|line| line["type"] == "entry")
            .map(|line| line["text"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(texts, vec!["first", "instead"]);
        assert!(read_lines(&whole)
            .iter()
            .filter(|line| line["type"] == "entry")
            .any(|line| line["text"] == "second" && line["onActiveBranch"] == false));
    }

    #[test]
    fn payload_newlines_do_not_break_line_framing() {
        let (dir, db) = session_db();
        say(&db, "message.completed", "line one\nline two\n{\"type\":\"header\"}");

        let export = export_forest(&dir, &db);
        let raw = std::fs::read_to_string(&export.path).unwrap();
        // Three lines and no more: a newline inside a payload must not forge a
        // fourth record — least of all one that looks like a second header.
        assert_eq!(raw.lines().count(), 3);
        let lines = read_lines(&export);
        assert_eq!(lines[1]["text"], "line one\nline two\n{\"type\":\"header\"}");
    }

    #[test]
    fn the_footer_digest_changes_when_an_entry_changes() {
        let (dir, db) = session_db();
        say(&db, "message.completed", "hello");
        let first = export_forest(&dir, &db);
        let repeat = export_forest(&dir, &db);
        assert_eq!(first.digest, repeat.digest, "the digest ignores the clock");

        say(&db, "message.completed", "and one more");
        let changed = export_forest(&dir, &db);
        assert_ne!(first.digest, changed.digest);
    }

    #[test]
    fn the_default_destination_lands_under_the_data_directory() {
        let (dir, db) = session_db();
        say(&db, "message.completed", "hello");
        let export = export_forest(&dir, &db);
        let path = PathBuf::from(&export.path);
        assert_eq!(path.parent().unwrap(), dir.path().join("exports"));
        assert_eq!(path.extension().unwrap(), "jsonl");
        assert!(path.exists());
    }

    #[test]
    fn two_exports_a_moment_apart_do_not_overwrite_each_other() {
        // Taking a second export to compare against the first is the normal
        // reason to take one, so the default filename must not collide.
        let (dir, db) = session_db();
        say(&db, "message.completed", "hello");
        let first = export_forest(&dir, &db);
        let second = export_forest(&dir, &db);
        assert_ne!(first.path, second.path);
        assert!(PathBuf::from(&first.path).exists());
        assert!(PathBuf::from(&second.path).exists());
    }

    #[test]
    fn a_supplied_destination_is_used_verbatim_and_its_parent_created() {
        let (dir, db) = session_db();
        say(&db, "message.completed", "hello");
        let target = dir.path().join("nested/deeper/session.jsonl");
        let export = export(
            &db,
            dir.path(),
            "s",
            ExportScope::Forest,
            true,
            Some(target.to_str().unwrap()),
        )
        .unwrap();
        assert_eq!(PathBuf::from(&export.path), target);
        assert!(target.exists());
    }

    #[test]
    fn a_relative_destination_is_refused_rather_than_resolved_against_the_cwd() {
        let (dir, db) = session_db();
        let error = export(
            &db,
            dir.path(),
            "s",
            ExportScope::Forest,
            true,
            Some("transcript.jsonl"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("absolute"), "{error}");
    }

    #[test]
    fn an_unknown_session_is_an_error_not_an_empty_file() {
        let (dir, db) = session_db();
        let error = export(&db, dir.path(), "nope", ExportScope::Forest, true, None).unwrap_err();
        assert!(error.to_string().contains("unknown session"), "{error}");
        assert!(!dir.path().join("exports").exists());
    }

    #[test]
    fn the_entry_lines_reproduce_the_rows_they_came_from() {
        // The round trip the export exists for: read the file back and assert
        // it is the forest, not a summary of it. An export that quietly drops
        // a field is worse than no export, because it reads as complete.
        let (dir, db) = session_db();
        say(&db, "turn.started", "");
        say(&db, "reasoning.completed", "Check the lock order.");
        say(&db, "message.completed", "Fixed.");
        say(&db, "usage.updated", "");

        let export = export_forest(&dir, &db);
        let lines = read_lines(&export);
        let exported = lines
            .iter()
            .filter(|line| line["type"] == "entry")
            .collect::<Vec<_>>();
        let stored = store::session_entries(&db, "s").unwrap();

        assert_eq!(exported.len(), stored.len());
        for (line, entry) in exported.iter().zip(stored.iter()) {
            assert_eq!(line["entryId"], entry.id);
            assert_eq!(line["sequence"], entry.sequence);
            assert_eq!(line["kind"], entry.kind);
            assert_eq!(line["createdAt"], entry.created_at);
            assert_eq!(line["contextVisibility"], entry.context_visibility);
            assert_eq!(
                line["parentEntryId"],
                entry.parent_entry_id.clone().map_or(Value::Null, Value::String)
            );
            // The stored document, whole and unaltered.
            assert_eq!(line["payload"], entry.payload);
        }
    }

    #[test]
    fn a_session_with_no_entries_still_exports_a_readable_file() {
        let (dir, db) = session_db();
        let export = export_forest(&dir, &db);
        let lines = read_lines(&export);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["type"], "header");
        assert_eq!(lines[1]["entryCount"], 0);
    }
}

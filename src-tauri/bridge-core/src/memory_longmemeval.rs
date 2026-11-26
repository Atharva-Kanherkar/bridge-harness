//! LongMemEval (Wu et al., ICLR 2025) on Bridge's real memory path.
//!
//! Each question gets an empty account scope. Its haystack sessions become
//! Bridge chats, oldest first, with their recorded dates. After each one,
//! production extraction runs on the eval profile (the same hidden tool-free
//! turn, prompt, digest and gate a real chat gets), every proposal is approved,
//! as a person who accepts everything would, and the new records take the
//! session's date. The question is then asked in a real chat on the same
//! profile: the memory packet rides its session-context frame and the `bridge`
//! tool server offers `memory_search` (and, under `LME_RECALL=full`, the
//! `chats_search` / `chats_read` history tools over the haystack chats).
//!
//! One JSON line per question goes to `LME_OUT`; `scripts/longmemeval/judge.py`
//! scores them with the benchmark's own judge prompts.
//!
//! ```text
//! BRIDGE_VISUALIZE_MCP_BIN=target/debug/bridged \
//! BRIDGE_MEMORY_EVAL_HARNESS=claude BRIDGE_MEMORY_EVAL_MODEL=claude-sonnet-5-5 \
//! LME_DATA=longmemeval_oracle.json LME_IDS=ids.txt LME_OUT=out.jsonl LME_SHARD=0/4 \
//! cargo test -p bridge-core --lib memory_longmemeval -- --ignored --nocapture
//! ```
//!
//! `LME_CACHE` names a directory of extraction answers keyed by profile,
//! prompt and digest, so a rerun that changes only retrieval or answering
//! spends nothing on extraction.

use std::collections::BTreeSet;
use std::io::{BufRead, Write};
use std::time::{Duration, Instant};

use chrono::{NaiveDateTime, TimeZone, Utc};
use rusqlite::{params, Connection};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::adapters::{ShutdownReason, StartRequest, TurnContext};
use bridge_protocol::messages::ACCOUNT_MEMORY_SCOPE;

#[derive(Deserialize)]
struct Turn {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct Question {
    question_id: String,
    question_type: String,
    question: Value,
    answer: Value,
    question_date: String,
    haystack_dates: Vec<String>,
    haystack_session_ids: Vec<String>,
    haystack_sessions: Vec<Vec<Turn>>,
}

fn harness() -> String {
    std::env::var("BRIDGE_MEMORY_EVAL_HARNESS").unwrap_or_else(|_| "claude".into())
}

fn model() -> Option<String> {
    std::env::var("BRIDGE_MEMORY_EVAL_MODEL").ok().filter(|model| !model.trim().is_empty())
}

fn text_of(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// `2023/04/10 (Mon) 17:50` as RFC 3339.
fn instant(raw: &str) -> String {
    let cleaned: String = {
        let mut parts = raw.split_whitespace().filter(|part| !part.starts_with('('));
        let day = parts.next().unwrap_or_default();
        let time = parts.next().unwrap_or("00:00");
        format!("{day} {time}")
    };
    NaiveDateTime::parse_from_str(&cleaned, "%Y/%m/%d %H:%M")
        .map(|naive| Utc.from_utc_datetime(&naive).to_rfc3339())
        .unwrap_or_else(|_| Utc::now().to_rfc3339())
}

fn cached_extraction(
    registry: &crate::adapters::AdapterRegistry,
    scratch: &std::path::Path,
    instructions: &str,
    digest: &str,
) -> Result<(String, i64, i64, bool), String> {
    let key = format!(
        "{:x}",
        Sha256::digest(format!("{}\0{}\0{instructions}\0{digest}", harness(), model().unwrap_or_default()).as_bytes())
    );
    let cache = std::env::var("LME_CACHE").ok().map(std::path::PathBuf::from);
    if let Some(path) = cache.as_ref().map(|dir| dir.join(&key)) {
        if let Ok(text) = std::fs::read_to_string(&path) {
            return Ok((text, 0, 0, true));
        }
    }
    let mut last = String::new();
    for _ in 0..3 {
        match crate::title_jobs_live::one_hidden_turn(registry, scratch, &harness(), model().as_deref(), instructions, digest, None) {
            Ok(generation) => {
                if let Some(dir) = cache.as_ref() {
                    let _ = std::fs::create_dir_all(dir);
                    let _ = std::fs::write(dir.join(&key), &generation.text);
                }
                return Ok((generation.text, generation.tokens, generation.spend_microusd, false));
            }
            Err(error) => {
                last = error;
                std::thread::sleep(Duration::from_secs(5));
            }
        }
    }
    Err(last)
}

/// Empty every table a question writes, so the next one starts clean.
fn reset(db: &Connection) {
    db.execute_batch(
        "PRAGMA foreign_keys=OFF;
         DELETE FROM memory_records;
         DELETE FROM memory_extraction_runs;
         DELETE FROM session_entries;
         DELETE FROM sessions;
         PRAGMA foreign_keys=ON;",
    )
    .expect("reset the scratch database");
}

#[derive(Default)]
struct Extracted {
    calls: usize,
    cached: usize,
    tokens: i64,
    spend: i64,
    errors: Vec<String>,
}

/// One haystack session through production extraction, every proposal
/// approved, the records dated to the session.
fn ingest(
    db: &Connection,
    registry: &crate::adapters::AdapterRegistry,
    scratch: &std::path::Path,
    session_id: &str,
    at: &str,
    turns: &[Turn],
    out: &mut Extracted,
) {
    db.execute(
        "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind,title,started_at)
         VALUES(?1,NULL,?2,?1,'idle','reported','direct',?1,?3)",
        params![session_id, harness(), at],
    )
    .unwrap();
    for turn in turns {
        let kind = if turn.role == "user" { "user.message" } else { "assistant.message" };
        let entry = crate::store::append_session_entry(db, session_id, None, kind, &json!({"text": turn.content}), None, "eligible", None).unwrap();
        db.execute("UPDATE session_entries SET created_at=?2 WHERE id=?1", params![entry.id, at]).unwrap();
    }
    let instructions = crate::memory_extraction::extraction_instructions();
    let mode = crate::memory_extraction::settings(db, ACCOUNT_MEMORY_SCOPE).unwrap().mode;
    for _ in 0..20 {
        let Some(digest) = crate::memory_extraction::build_digest(db, ACCOUNT_MEMORY_SCOPE, session_id).unwrap() else { break };
        if digest.has_evidence {
            match cached_extraction(registry, scratch, &instructions, &digest.text) {
                Ok((text, tokens, spend, cached)) => {
                    out.calls += 1;
                    out.cached += usize::from(cached);
                    out.tokens += tokens;
                    out.spend += spend;
                    if let Err(error) = crate::memory_extraction::gate_and_insert(db, ACCOUNT_MEMORY_SCOPE, session_id, &mode, &text, &digest.known) {
                        out.errors.push(format!("gate: {error}"));
                    }
                }
                Err(error) => out.errors.push(format!("extract: {error}")),
            }
        }
        let proposed: Vec<String> = db
            .prepare("SELECT id FROM memory_records WHERE status='proposed'")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for id in proposed {
            if let Err(error) = crate::memory_ledger::approve(db, &id) {
                out.errors.push(format!("approve: {error}"));
            }
        }
        // The watermark a settled production run leaves.
        db.execute(
            "INSERT INTO memory_extraction_runs(id,scope_key,session_id,status,created_at,updated_at,due_at,through_entry_id)
             VALUES(?1,?2,?3,'completed',?4,?4,?4,?5)",
            params![uuid::Uuid::new_v4().to_string(), ACCOUNT_MEMORY_SCOPE, session_id, at, digest.through_entry_id],
        )
        .unwrap();
        if !digest.more {
            break;
        }
    }
    db.execute(
        "UPDATE memory_records SET created_at=?2, updated_at=?2, valid_from=?2 WHERE source_session_id=?1",
        params![session_id, at],
    )
    .unwrap();
    db.execute(
        "UPDATE memory_records SET valid_to=?2, updated_at=?2
         WHERE status='superseded' AND superseded_by IN (SELECT id FROM memory_records WHERE source_session_id=?1)",
        params![session_id, at],
    )
    .unwrap();
}

struct Answer {
    reply: String,
    tools: Vec<Value>,
    error: Option<String>,
}

fn ask(
    core: &std::sync::Arc<crate::runtime::BridgeCore>,
    registry: &crate::adapters::AdapterRegistry,
    cwd: &std::path::Path,
    session: &str,
    prompt: &str,
) -> (Answer, usize) {
    let (frame, packet_chars) = {
        let db = core.db.lock().unwrap();
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,kind) VALUES(?1,NULL,?2,'ask','ready','direct')",
            params![session, harness()],
        )
        .unwrap();
        let guidance = crate::memory_tools::session_guidance(&db, session).unwrap_or_default();
        let packet = crate::memory_packet::for_compile(&db, session).unwrap();
        let chars = packet.as_deref().map_or(0, str::len);
        (crate::session_context::build(guidance, packet.as_deref()), chars)
    };
    let cwd_text = cwd.to_string_lossy().to_string();
    let model = model();
    let started = registry.start(
        &harness(),
        StartRequest {
            session_id: Some(session),
            cwd: &cwd_text,
            model: model.as_deref(),
            effort: None,
            speed: None,
            instructions: None,
            write_mode: Some(crate::delegation::WriteMode::Full),
            read_only_sandbox: None,
            briefing: None,
            interactive: true,
            account: None,
            on_progress: None,
        },
    );
    let started = match started {
        Ok(started) => started,
        Err(error) => return (Answer { reply: String::new(), tools: Vec::new(), error: Some(format!("start: {error}")) }, packet_chars),
    };
    let mut runtime = started.runtime;
    let context = TurnContext { session: frame.as_ref().map(crate::session_context::SessionContext::text), credentials: None };
    if let Err(error) = runtime.send_turn_with_context(prompt, context) {
        runtime.stop(ShutdownReason::Completed);
        return (Answer { reply: String::new(), tools: Vec::new(), error: Some(format!("send: {error}")) }, packet_chars);
    }
    let mut answer = Answer { reply: String::new(), tools: Vec::new(), error: None };
    let deadline = Instant::now() + Duration::from_secs(360);
    let mut lines = started.reader.lines();
    let mut finished = false;
    while Instant::now() < deadline {
        let Some(Ok(line)) = lines.next() else { break };
        let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
        for event in registry.normalize(&harness(), &value) {
            if event.kind == "tool.started" {
                answer.tools.push(json!({"tool": event.title, "input": event.data}));
            }
            if event.kind == "message.completed" && event.role.as_deref() == Some("assistant") {
                answer.reply.push_str(event.text.as_deref().unwrap_or_default());
                answer.reply.push('\n');
            }
            if event.kind == "turn.failed" {
                answer.error = Some(format!("turn failed: {}", event.text.clone().unwrap_or_default()));
            }
            if event.kind == "turn.completed" || event.kind == "turn.failed" {
                finished = true;
            }
        }
        if finished {
            break;
        }
    }
    if !finished && answer.error.is_none() {
        answer.error = Some("no turn.completed before the deadline".into());
    }
    runtime.stop(ShutdownReason::Completed);
    (answer, packet_chars)
}

#[test]
#[ignore = "runs real provider turns over LongMemEval"]
fn live_longmemeval() {
    let data = std::env::var("LME_DATA").expect("set LME_DATA to a LongMemEval json file");
    let out_path = std::env::var("LME_OUT").expect("set LME_OUT to a jsonl path");
    assert!(std::env::var_os(crate::mcp_apps::BIN_ENV).is_some(), "point {} at a bridged built from this tree", crate::mcp_apps::BIN_ENV);
    let wanted: Option<Vec<String>> = std::env::var("LME_IDS").ok().map(|path| {
        std::fs::read_to_string(path).unwrap().lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_owned).collect()
    });
    let (shard, shards) = std::env::var("LME_SHARD")
        .ok()
        .and_then(|raw| raw.split_once('/').map(|(a, b)| (a.parse::<usize>().unwrap(), b.parse::<usize>().unwrap())))
        .unwrap_or((0, 1));
    let recall = std::env::var("LME_RECALL").unwrap_or_else(|_| "full".into());
    let done: BTreeSet<String> = std::fs::read_to_string(&out_path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|row| row["error"].is_null())
        .filter_map(|row| row["question_id"].as_str().map(str::to_owned))
        .collect();

    let all: Vec<Question> = serde_json::from_reader(std::io::BufReader::new(std::fs::File::open(&data).unwrap())).expect("LongMemEval json");
    let mut questions: Vec<Question> = match &wanted {
        Some(ids) => {
            let mut by_id: std::collections::HashMap<String, Question> = all.into_iter().map(|q| (q.question_id.clone(), q)).collect();
            ids.iter().filter_map(|id| by_id.remove(id)).collect()
        }
        None => all,
    };
    questions = questions.into_iter().enumerate().filter(|(index, _)| index % shards == shard).map(|(_, q)| q).collect();

    let dir = tempfile::tempdir().unwrap();
    let core = std::sync::Arc::new(crate::runtime::BridgeCore::for_tests(dir.path()));
    crate::notes::tool::install(&core);
    crate::notes::tool::serve_listener().expect("the chat-tools listener");
    let registry = std::sync::Arc::new(crate::adapters::AdapterRegistry::built_in().expect("adapters"));
    std::thread::sleep(Duration::from_secs(8));
    let scratch = dir.path().join("extraction");
    let cwd = dir.path().join("cwd");
    std::fs::create_dir_all(&cwd).unwrap();

    for question in &questions {
        if done.contains(&question.question_id) {
            continue;
        }
        let started = Instant::now();
        let mut extracted = Extracted::default();
        let mut order: Vec<usize> = (0..question.haystack_sessions.len()).collect();
        order.sort_by_key(|&index| instant(&question.haystack_dates[index]));
        {
            let db = core.db.lock().unwrap();
            reset(&db);
            for (position, &index) in order.iter().enumerate() {
                let session_id = format!("hay-{position:03}-{}", question.haystack_session_ids[index]);
                ingest(&db, &registry, &scratch, &session_id, &instant(&question.haystack_dates[index]), &question.haystack_sessions[index], &mut extracted);
            }
            if recall == "ledger" {
                db.execute_batch("DELETE FROM session_entries; DELETE FROM sessions;").unwrap();
            }
        }
        let extract_seconds = started.elapsed().as_secs_f64();
        let records: Vec<Value> = {
            let db = core.db.lock().unwrap();
            let mut statement = db.prepare("SELECT kind, body, valid_from FROM memory_records WHERE status='active' ORDER BY valid_from").unwrap();
            let rows = statement
                .query_map([], |row| Ok(json!({"kind": row.get::<_, String>(0)?, "body": row.get::<_, String>(1)?, "at": row.get::<_, String>(2)?})))
                .unwrap();
            rows.map(Result::unwrap).collect()
        };
        let asked = Instant::now();
        let prompt = format!("(Today is {}.) {}", question.question_date, text_of(&question.question));
        let session = format!("ask-{}", question.question_id);
        let (answer, packet_chars) = ask(&core, &registry, &cwd, &session, &prompt);
        let row = json!({
            "question_id": question.question_id,
            "question_type": question.question_type,
            "question": text_of(&question.question),
            "answer": text_of(&question.answer),
            "question_date": question.question_date,
            "hypothesis": answer.reply.trim(),
            "tools": answer.tools,
            "harness": harness(),
            "model": model(),
            "recall": recall,
            "sessions": question.haystack_sessions.len(),
            "active_records": records.len(),
            "records": records,
            "packet_chars": packet_chars,
            "extraction_calls": extracted.calls,
            "extraction_cached": extracted.cached,
            "extraction_tokens": extracted.tokens,
            "extraction_spend_microusd": extracted.spend,
            "extraction_errors": extracted.errors,
            "seconds_extract": extract_seconds,
            "seconds_answer": asked.elapsed().as_secs_f64(),
            "error": answer.error,
        });
        println!(
            "LME {} {} records {} tools {} {:.0}s+{:.0}s {}",
            question.question_id,
            question.question_type,
            records.len(),
            answer.tools.len(),
            extract_seconds,
            asked.elapsed().as_secs_f64(),
            answer.reply.trim().chars().take(120).collect::<String>().replace('\n', " ")
        );
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&out_path).unwrap();
        writeln!(file, "{row}").unwrap();
    }
}

//! Property tests for the model-switch handoff: the tool facts, the Bridge
//! facts block, the tail and `chats_history`.
//!
//! Each property opens one database and clears the chat between cases. What
//! they pin, for any branch of messages and tool calls with any text and any
//! payload shape, and any window:
//!
//! - the handoff never panics, is byte-identical when built twice, stays
//!   within the incoming model's budget (and the 96 KiB cap), keeps a model
//!   summary as its first line, carries no more messages than were stored,
//!   and every `tools:` line is at most 480 bytes;
//! - tool-fact extraction survives any JSON and its lines stay bounded;
//! - the facts block keeps its caps, and every link is a whole PR, MR or issue
//!   URL with no query or fragment;
//! - a `chats_history` page holds only the caller's entries before `before`,
//!   oldest first, within the limit and the page size, and never a planted
//!   credential.

use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};
use rusqlite::{params, Connection};
use serde_json::{json, Map, Value};

use crate::handoff_facts::{self, MAX_FACTS_BYTES, MAX_FILES, MAX_LINKS};
use crate::model::SessionEntry;
use crate::restoration::{self, MAX_RESTORATION_BUDGET_BYTES};
use crate::session_forest::{EntryKind, SessionForest};
use crate::tool_facts::{self, MAX_LINE_BYTES};

const CHAT: &str = "chat";
const OTHER: &str = "other";
const ENVELOPE: &str = "Bridge checkpoint-restoration context (stored history, not native provider resume):\n";
/// A credential of a shape every scrubber detects, planted in tool calls.
const TOKEN: &str = "ghp_Zq9Xw8Vu7Ts6Rp5On4MlKj3Ih2Gf1Ed0CbAz";

fn property_db() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::store::open(&dir.path().join("bridge.db")).unwrap();
    for id in [CHAT, OTHER] {
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind) VALUES(?1,NULL,'claude','chat','idle','reported','direct')",
            params![id],
        )
        .unwrap();
    }
    (dir, db)
}

/// A new, empty top-level chat for one case.
fn fresh_chat(db: &Connection) -> String {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = format!("case-{}", NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    db.execute(
        "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind) VALUES(?1,NULL,'claude','chat','idle','reported','direct')",
        params![id],
    )
    .unwrap();
    id
}

fn runner(cases: u32) -> TestRunner {
    TestRunner::new(Config { cases, failure_persistence: None, ..Config::default() })
}

/// Text that is any Unicode, sometimes long, sometimes holding a link, a
/// credential or a path.
fn text() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => ".{0,200}",
        1 => ".{0,40}".prop_map(|prefix| format!("{prefix} https://github.com/acme/app/pull/12?x=1#y")),
        1 => Just(format!("GH_TOKEN={TOKEN} gh pr list")),
        1 => (0usize..20_000).prop_map(|n| "ü界x".repeat(n / 3)),
        1 => Just("/bin/zsh -lc 'cargo test'".to_owned()),
        1 => Just(String::new()),
    ]
}

/// JSON shaped like adapter payloads: their keys, at any depth, holding any
/// value.
fn json_value() -> impl Strategy<Value = Value> {
    const KEYS: &[&str] = &[
        "data", "input", "command", "text", "title", "changes", "path", "file_path", "filePath", "state", "exitCode",
        "exit_code", "is_error", "status", "output", "aggregatedOutput", "content", "name", "tool", "itemId",
        "metadata", "exit", "query", "url", "type", "result", "summary", "filesChanged", "filesTouched",
    ];
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|n| json!(n)),
        text().prop_map(Value::String),
    ];
    leaf.prop_recursive(4, 48, 6, move |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..5).prop_map(Value::Array),
            prop::collection::vec((prop::sample::select(KEYS), inner), 0..6)
                .prop_map(|pairs| Value::Object(pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect::<Map<_, _>>())),
        ]
    })
}

#[derive(Debug, Clone)]
enum Step {
    Say(EntryKind, String),
    Checkpoint(String),
    Tool(&'static str, String, Value),
}

fn step() -> impl Strategy<Value = Step> {
    let item = prop::sample::select(vec!["a", "b", "c", "d"]).prop_map(str::to_owned);
    prop_oneof![
        3 => text().prop_map(|t| Step::Say(EntryKind::UserMessage, t)),
        3 => text().prop_map(|t| Step::Say(EntryKind::AssistantMessage, t)),
        1 => text().prop_map(Step::Checkpoint),
        4 => (
            prop::sample::select(vec!["command.started", "command.completed", "file_change.completed", "tool.completed", "tool.started", "command.output_delta"]),
            item,
            json_value(),
        )
            .prop_map(|(kind, item, data)| Step::Tool(kind, item, data)),
    ]
}

fn apply(db: &Connection, session: &str, steps: &[Step]) {
    let forest = SessionForest::new(db);
    for step in steps {
        match step {
            Step::Say(kind, text) if !text.trim().is_empty() => {
                forest.append(session, kind.clone(), json!({"text": text})).unwrap();
            }
            Step::Checkpoint(summary) if !summary.trim().is_empty() => {
                forest
                    .append(session, EntryKind::Checkpoint, json!({"schemaVersion": 1, "summary": summary, "decisions": ["d"], "openWork": ["o"]}))
                    .unwrap();
            }
            Step::Tool(kind, item, data) => {
                let mut event = crate::agent::NormalizedEvent::new(kind);
                event.item_id = Some(item.clone());
                event.text = data.get("text").and_then(Value::as_str).map(str::to_owned);
                event.data = data.clone();
                // An event the envelope refuses is one no adapter could write.
                let _ = crate::store::session_event(db, session, &event, &json!({}));
            }
            _ => {}
        }
    }
}

#[test]
fn the_handoff_keeps_its_bounds_for_any_branch_and_window() {
    let (_dir, db) = property_db();
    runner(96)
        .run(
            &(prop::collection::vec(step(), 0..40), prop_oneof![Just(1i64), 1i64..2_000_000, Just(200_000i64)]),
            |(steps, window)| {
                let chat = fresh_chat(&db);
                apply(&db, &chat, &steps);
                let first = restoration::restoration_handoff(&db, &chat, window).unwrap();
                let again = restoration::restoration_handoff(&db, &chat, window).unwrap();
                prop_assert_eq!(&first, &again, "deterministic");
                let Some(handoff) = first else { return Ok(()) };
                let body = handoff.text.strip_prefix(ENVELOPE).expect("the envelope heads every handoff");
                prop_assert!(body.len() <= MAX_RESTORATION_BUDGET_BYTES, "{} > cap", body.len());
                if handoff.header.is_none() {
                    let budget = restoration::restoration_budget_bytes(window);
                    prop_assert!(body.len() <= budget, "{} > budget {budget}", body.len());
                } else {
                    prop_assert!(body.starts_with("checkpoint: "), "the model summary comes first: {:?}", &body[..body.len().min(80)]);
                }
                prop_assert!(handoff.carried_entries <= handoff.eligible_entries);
                for line in body.lines().filter(|line| line.starts_with("tools: ")) {
                    prop_assert!(line.len() <= MAX_LINE_BYTES, "{}", line.len());
                    prop_assert!(!line.contains(TOKEN), "{line}");
                }
                let facts = body
                    .lines()
                    .filter(|line| ["goal: ", "started with: ", "files touched: ", "links: ", "earlier: ", "tool output: "].iter().any(|p| line.starts_with(p)))
                    .map(|line| line.len() + 1)
                    .sum::<usize>();
                prop_assert!(facts <= MAX_FACTS_BYTES);
                Ok(())
            },
        )
        .unwrap();
}

fn entry(sequence: i64, kind: &str, payload: Value) -> SessionEntry {
    SessionEntry {
        id: format!("e{sequence}"),
        session_id: CHAT.into(),
        parent_entry_id: None,
        sequence,
        semantic_schema_version: crate::model::SEMANTIC_EVENT_SCHEMA_VERSION,
        kind: kind.into(),
        payload,
        provider_event_id: None,
        context_visibility: "eligible".into(),
        token_estimate: None,
        created_at: "now".into(),
    }
}

#[test]
fn tool_facts_survive_any_payload_and_stay_bounded() {
    runner(512)
        .run(
            &prop::collection::vec(
                (prop::sample::select(vec!["command.started", "command.completed", "file_change.completed", "tool.completed", "file_change.started"]), json_value()),
                0..12,
            ),
            |payloads| {
                let entries = payloads
                    .iter()
                    .enumerate()
                    .map(|(index, (kind, data))| entry(index as i64, kind, json!({"itemId": "same", "title": data.get("title"), "data": data, "text": data.get("text")})))
                    .collect::<Vec<_>>();
                let index = tool_facts::StartedIndex::new(entries.iter());
                let facts = entries.iter().filter_map(|entry| tool_facts::fact(entry, &index)).collect::<Vec<_>>();
                for fact in &facts {
                    let rendered = fact.render();
                    prop_assert!(rendered.len() <= 4 * 200, "{}", rendered.len());
                    prop_assert!(!rendered.contains(TOKEN));
                }
                if let Some(line) = tool_facts::line(&facts) {
                    prop_assert!(line.len() <= MAX_LINE_BYTES, "{}", line.len());
                    prop_assert!(line.starts_with("tools: "));
                } else {
                    prop_assert!(facts.is_empty());
                }
                for (_, data) in &payloads {
                    if let Some(output) = tool_facts::output(&json!({"data": data})) {
                        prop_assert!(!output.is_empty() && output.trim() == output);
                    }
                }
                Ok(())
            },
        )
        .unwrap();
}

#[test]
fn the_facts_block_keeps_its_caps_and_links_are_whole() {
    let link = regex::Regex::new(r"^https://(github\.com/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+/(pull|issues)/[0-9]+|gitlab\.com/.+/-/(merge_requests|issues)/[0-9]+)$").unwrap();
    runner(256)
        .run(
            &prop::collection::vec(
                (prop::sample::select(vec!["user.message", "assistant.message", "worker.result", "command.completed", "file_change.completed", "checkpoint", "artifact.created"]), json_value(), text()),
                0..30,
            ),
            |rows| {
                let branch = rows
                    .iter()
                    .enumerate()
                    .map(|(index, (kind, data, text))| {
                        entry(index as i64, kind, json!({"text": text, "summary": text, "data": data, "itemId": format!("i{index}"), "filesChanged": data.get("filesChanged"), "filesTouched": data.get("filesTouched"), "path": data.get("path")}))
                    })
                    .collect::<Vec<_>>();
                let facts = handoff_facts::collect(&branch);
                prop_assert!(facts.files.len() <= MAX_FILES);
                prop_assert!(facts.links.len() <= MAX_LINKS);
                for found in &facts.links {
                    prop_assert!(link.is_match(found), "{found}");
                }
                if let Some(goal) = &facts.goal {
                    prop_assert!(goal.len() <= handoff_facts::GOAL_BYTES && !goal.contains('\n') && !goal.contains(TOKEN));
                }
                let lines = handoff_facts::bound(facts.lines(false, Some(handoff_facts::earlier_line(9, 9))), MAX_FACTS_BYTES);
                prop_assert!(lines.iter().map(|line| line.len() + 1).sum::<usize>() <= MAX_FACTS_BYTES);
                Ok(())
            },
        )
        .unwrap();
}

#[test]
fn a_history_page_is_the_callers_own_bounded_and_ordered() {
    let (_dir, db) = property_db();
    runner(64)
        .run(
            &(prop::collection::vec(step(), 0..50), prop::option::of(-5i64..80), prop::option::of(0usize..80)),
            |(steps, before, limit)| {
                let chat = fresh_chat(&db);
                apply(&db, &chat, &steps);
                apply(&db, OTHER, &[Step::Say(EntryKind::UserMessage, "OTHER CHAT ONLY".into())]);
                let mut args = Map::new();
                if let Some(before) = before {
                    args.insert("before".into(), json!(before));
                }
                if let Some(limit) = limit {
                    args.insert("limit".into(), json!(limit));
                }
                let page = crate::history_tools::run(&db, &chat, crate::history_tools::HISTORY, &Value::Object(args), chrono::Utc::now()).unwrap();
                let serialized = page.to_string();
                prop_assert!(serialized.len() < crate::tool_limit::BOUNDED_CEILING, "{}", serialized.len());
                prop_assert!(!serialized.contains("OTHER CHAT ONLY"));
                prop_assert!(!serialized.contains(TOKEN));
                let sequences = page["entries"].as_array().unwrap().iter().map(|item| item["sequence"].as_i64().unwrap()).collect::<Vec<_>>();
                prop_assert!(sequences.windows(2).all(|pair| pair[0] < pair[1]), "{sequences:?}");
                prop_assert!(sequences.len() <= limit.unwrap_or(20).clamp(1, 50));
                if let Some(before) = before {
                    prop_assert!(sequences.iter().all(|sequence| *sequence < before));
                }
                if let Some(next) = page["nextBefore"].as_i64() {
                    prop_assert_eq!(Some(next), sequences.first().copied());
                }
                Ok(())
            },
        )
        .unwrap();
}

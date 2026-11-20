//! The model-switch handoff on raw bytes: any branch of messages, checkpoints
//! and tool calls in any payload shape, and any window. The handoff never
//! panics, stays within the incoming model's budget (and the 96 KiB cap),
//! keeps every `tools:` line at 480 bytes, and never carries a credential
//! planted in a tool call.
#![no_main]

use std::cell::{Cell, RefCell};

use bridge_core::{restoration, session_forest::{EntryKind, SessionForest}, tool_facts};
use libfuzzer_sys::fuzz_target;
use rusqlite::Connection;
use serde_json::{json, Value};

const TOKEN: &str = "ghp_Zq9Xw8Vu7Ts6Rp5On4MlKj3Ih2Gf1Ed0CbAz";
const ENVELOPE: &str = "Bridge checkpoint-restoration context (stored history, not native provider resume):\n";
const KINDS: [&str; 8] = [
    "user.message", "assistant.message", "checkpoint", "command.started", "command.completed",
    "file_change.completed", "tool.completed", "worker.result",
];

thread_local! {
    static DB: RefCell<Option<(tempfile::TempDir, Connection)>> = const { RefCell::new(None) };
    static RUNS: Cell<usize> = const { Cell::new(0) };
}

/// A fresh chat in a database that is replaced every few thousand inputs,
/// so throughput goes to the handoff and not to migrations or a growing file.
fn with_chat(run: impl FnOnce(&Connection, &str)) {
    let count = RUNS.with(|runs| {
        runs.set(runs.get() + 1);
        runs.get()
    });
    DB.with(|cell| {
        let mut cell = cell.borrow_mut();
        if cell.is_none() || count % 4000 == 0 {
            let dir = tempfile::tempdir().unwrap();
            let db = bridge_core::store::open(&dir.path().join("bridge.db")).unwrap();
            *cell = Some((dir, db));
        }
        let (_, db) = cell.as_ref().unwrap();
        let chat = format!("chat-{count}");
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind) VALUES(?1,NULL,'claude','chat','idle','reported','direct')",
            [&chat],
        )
        .unwrap();
        run(db, &chat);
    });
}

fn payload(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(bytes).into_owned()))
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }
    let window = i64::from(u32::from_le_bytes([data[0], data[1], data[2], data[3]]) % 2_000_000);
    let parts = data[4..].split(|byte| *byte == 0xFF).take(64).collect::<Vec<_>>();
    with_chat(|db, chat| {
        let forest = SessionForest::new(db);
        for (index, part) in parts.iter().enumerate() {
            let Some((&selector, rest)) = part.split_first() else { continue };
            let kind = KINDS[usize::from(selector) % KINDS.len()];
            let body = payload(rest);
            let text = body.as_str().map(str::to_owned).unwrap_or_else(|| body.to_string());
            match kind {
                "user.message" | "assistant.message" => {
                    let kind = if kind == "user.message" { EntryKind::UserMessage } else { EntryKind::AssistantMessage };
                    let _ = forest.append(chat, kind, json!({"text": text}));
                }
                "checkpoint" => {
                    let _ = forest.append(chat, EntryKind::Checkpoint, json!({"schemaVersion": 1, "summary": text, "openWork": [text]}));
                }
                "worker.result" => {
                    let _ = forest.append(chat, EntryKind::WorkerResult, json!({"status": "done", "summary": text, "filesChanged": body.get("filesChanged")}));
                }
                _ => {
                    let mut event = bridge_core::agent::NormalizedEvent::new(kind);
                    event.item_id = Some(format!("item-{}", index % 3));
                    let mut data = if body.is_object() { body.clone() } else { json!({"command": text}) };
                    if selector & 0x80 != 0 {
                        data["command"] = json!(format!("GH_TOKEN={TOKEN} {text}"));
                    }
                    event.text = data.get("output").and_then(Value::as_str).map(str::to_owned);
                    event.data = data;
                    let _ = bridge_core::store::session_event(db, chat, &event, &json!({}));
                }
            }
        }
        let Ok(Some(handoff)) = restoration::restoration_handoff(db, chat, window) else { return };
        let body = handoff.text.strip_prefix(ENVELOPE).expect("the envelope heads every handoff");
        assert!(body.len() <= restoration::MAX_RESTORATION_BUDGET_BYTES, "{} over the cap", body.len());
        if handoff.header.is_none() {
            assert!(body.len() <= restoration::restoration_budget_bytes(window), "{} over the budget", body.len());
        }
        assert!(handoff.carried_entries <= handoff.eligible_entries);
        for line in body.lines().filter(|line| line.starts_with("tools: ")) {
            assert!(line.len() <= tool_facts::MAX_LINE_BYTES, "{}", line.len());
            assert!(!line.contains(&TOKEN[4..16]), "a planted credential in {line}");
        }
    });
});

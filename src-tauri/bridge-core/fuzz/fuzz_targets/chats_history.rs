//! `chats_history` on raw bytes as arguments: bytes that are not JSON become a
//! string, any JSON is passed through. It never panics, refuses only with the
//! documented statuses, and a page holds only the caller's entries before
//! `before`, oldest first, under the server's ceiling, with no planted
//! credential and nothing from another chat.
#![no_main]

use std::sync::{Mutex, OnceLock};

use bridge_core::{history_tools, session_forest::{EntryKind, SessionForest}};
use libfuzzer_sys::fuzz_target;
use rusqlite::Connection;
use serde_json::{json, Value};

const TOKEN: &str = "ghp_Zq9Xw8Vu7Ts6Rp5On4MlKj3Ih2Gf1Ed0CbAz";

fn db() -> &'static Mutex<Connection> {
    static DB: OnceLock<Mutex<Connection>> = OnceLock::new();
    DB.get_or_init(|| {
        let dir = Box::leak(Box::new(tempfile::tempdir().unwrap()));
        let db = bridge_core::store::open(&dir.path().join("bridge.db")).unwrap();
        for id in ["chat", "other", "child"] {
            db.execute(
                "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind,parent_session_id) VALUES(?1,NULL,'claude','chat','idle','reported','direct',CASE WHEN ?1='child' THEN 'chat' END)",
                [id],
            )
            .unwrap();
        }
        let forest = SessionForest::new(&db);
        forest.append("other", EntryKind::UserMessage, json!({"text": "OTHER CHAT ONLY"})).unwrap();
        for turn in 0..120 {
            let kind = if turn % 2 == 0 { EntryKind::UserMessage } else { EntryKind::AssistantMessage };
            forest.append("chat", kind, json!({"text": format!("turn {turn} {}", "ü界".repeat(turn * 7))})).unwrap();
            if turn % 5 == 0 {
                let mut event = bridge_core::agent::NormalizedEvent::new("command.completed");
                event.item_id = Some(format!("exec-{turn}"));
                event.text = Some(format!("output with {TOKEN} {}", "o".repeat(turn * 40)));
                event.data = json!({"command": format!("GH_TOKEN={TOKEN} gh pr list"), "exitCode": turn % 3});
                bridge_core::store::session_event(&db, "chat", &event, &json!({})).unwrap();
            }
        }
        Mutex::new(db)
    })
}

fuzz_target!(|data: &[u8]| {
    let Some((&selector, rest)) = data.split_first() else { return };
    let caller = ["chat", "other", "child", "missing"][usize::from(selector % 4)];
    let args: Value = serde_json::from_slice(rest).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(rest).into_owned()));
    let db = db().lock().unwrap();
    match history_tools::run(&db, caller, history_tools::HISTORY, &args, chrono::Utc::now()) {
        Ok(page) => {
            assert!(caller == "chat" || caller == "other", "{caller} must be refused");
            let text = page.to_string();
            assert!(text.len() < 32 * 1024, "{}", text.len());
            assert!(!text.contains(&TOKEN[4..16]), "a planted credential in a page");
            if caller == "chat" {
                assert!(!text.contains("OTHER CHAT ONLY"));
            }
            let sequences = page["entries"].as_array().unwrap().iter().map(|item| item["sequence"].as_i64().unwrap()).collect::<Vec<_>>();
            assert!(sequences.windows(2).all(|pair| pair[0] < pair[1]));
            assert!(sequences.len() <= 50);
            if let Some(before) = args.get("before").and_then(Value::as_i64) {
                assert!(sequences.iter().all(|sequence| *sequence < before));
            }
        }
        Err(error) => assert!([400, 404].contains(&error.status), "{} {}", error.status, error.message),
    }
});

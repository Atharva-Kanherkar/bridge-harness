// Shared fixture: one database for the whole fuzz run, reset per input, so
// throughput is spent on the gate rather than on migrations.
use std::sync::{Mutex, OnceLock};

use bridge_core::{memory_consolidation, memory_extraction};
use bridge_protocol::messages::ACCOUNT_MEMORY_SCOPE;
use rusqlite::Connection;

pub const CHAT: &str = "chat";

pub fn db() -> &'static Mutex<Connection> {
    static DB: OnceLock<Mutex<Connection>> = OnceLock::new();
    DB.get_or_init(|| {
        let dir = Box::leak(Box::new(tempfile::tempdir().unwrap()));
        let db = bridge_core::store::open(&dir.path().join("bridge.db")).unwrap();
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind)
             VALUES('chat',NULL,'claude','chat','idle','reported','direct')",
            [],
        )
        .unwrap();
        Mutex::new(db)
    })
}

pub fn reset(db: &Connection, max_records: i64, mode: &str) {
    db.execute_batch("DELETE FROM memory_records;").unwrap();
    memory_consolidation::update_settings(db, ACCOUNT_MEMORY_SCOPE, "off", None, None, Some(max_records), None, None).unwrap();
    let (harness, model) = if mode == memory_extraction::MODE_REMEMBER { (None, None) } else { (Some("claude"), Some("sonnet")) };
    memory_extraction::update_settings(db, ACCOUNT_MEMORY_SCOPE, mode, harness, model).unwrap();
}

pub fn rows(db: &Connection) -> Vec<(String, String)> {
    let mut statement = db.prepare("SELECT status, provenance FROM memory_records").unwrap();
    statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?))).unwrap().collect::<Result<_, _>>().unwrap()
}

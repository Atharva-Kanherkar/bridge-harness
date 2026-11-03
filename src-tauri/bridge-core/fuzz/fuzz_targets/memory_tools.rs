//! The agent memory tools on raw bytes: arguments that are not JSON become a
//! string, any JSON is passed through, and the tools never panic, answer only
//! the documented refusals, and write nothing but `agent_proposal` rows.
#![no_main]

mod common;

use bridge_core::{memory_extraction, memory_tools};
use libfuzzer_sys::fuzz_target;
use serde_json::Value;

fuzz_target!(|data: &[u8]| {
    let Some((&selector, rest)) = data.split_first() else { return };
    let tool = [memory_tools::PROPOSE, memory_tools::SEARCH, "memory_forget"][usize::from(selector % 3)];
    let mode = [memory_extraction::MODE_PROPOSE, memory_extraction::MODE_AUTO_APPLY, memory_extraction::MODE_REMEMBER][usize::from(selector / 3 % 3)];
    let may_propose = selector & 0x80 != 0;
    let args: Value = serde_json::from_slice(rest).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(rest).into_owned()));
    let db = common::db().lock().unwrap();
    common::reset(&db, 50, mode);
    match memory_tools::run(&db, common::CHAT, may_propose, tool, &args) {
        Ok(outcome) => assert_eq!(outcome.changed, common::rows(&db).len() == 1),
        Err(error) => {
            assert!([400, 403, 404, 409, 429].contains(&error.status), "{}", error.status);
            assert!(common::rows(&db).is_empty());
        }
    }
    for (status, provenance) in common::rows(&db) {
        assert_eq!((status.as_str(), provenance.as_str()), ("proposed", "agent_proposal"));
    }
});

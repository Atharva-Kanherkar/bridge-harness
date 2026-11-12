//! The extraction gate on raw bytes: whatever a model answers, the gate never
//! panics, writes only `proposed` / `model_proposal` rows in review-first
//! mode, and never takes the scope past its budget.
#![no_main]

mod common;

use bridge_core::{memory_consolidation, memory_extraction, memory_ledger};
use bridge_protocol::messages::ACCOUNT_MEMORY_SCOPE;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&budget, text)) = data.split_first() else { return };
    let budget = i64::from(budget % 8) + 1;
    let text = String::from_utf8_lossy(text);
    let db = common::db().lock().unwrap();
    common::reset(&db, budget, memory_extraction::MODE_PROPOSE);
    memory_ledger::save(&db, "Never push directly to main", None, None).unwrap();
    let before = common::rows(&db).len();
    let outcome = memory_extraction::gate_and_insert(&db, ACCOUNT_MEMORY_SCOPE, common::CHAT, memory_extraction::MODE_PROPOSE, &text);
    let rows = common::rows(&db);
    match outcome {
        Ok(report) => {
            assert_eq!(report.activated, 0);
            assert_eq!(rows.len() - before, report.written);
        }
        Err(_) => assert_eq!(rows.len(), before),
    }
    for (status, provenance) in rows {
        assert!(
            (status == "active" && provenance == "user_explicit") || (status == "proposed" && provenance == "model_proposal"),
            "{status}/{provenance}"
        );
    }
    assert!(memory_consolidation::held_records(&db, ACCOUNT_MEMORY_SCOPE).unwrap() <= budget);
});

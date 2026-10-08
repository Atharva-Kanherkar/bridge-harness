//! The repeat guard: a visual the user has already seen is not drawn again.
//!
//! A model that charts the same data on every follow-up ("thanks", "and what
//! about…") buries the answer under copies of one picture. The guard keeps a
//! fingerprint of the last [`REMEMBERED`] accepted specs — family, form and
//! data only, so a new title does not make old data new — and refuses a
//! match unless the spec says `redraw: true`, which the tool description
//! reserves for a user who asked to see it again or to change it.
//!
//! The server process lives as long as the harness session's MCP connection,
//! so this is per session for every harness Bridge attaches it to.

use std::collections::VecDeque;

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::spec::SpecError;

pub const REMEMBERED: usize = 10;

#[derive(Debug, Default)]
pub struct RepeatGuard {
    recent: VecDeque<String>,
}

/// What makes two visuals the same picture: each block's family, form and
/// data, in order. Titles, captions, colours and follow-ups are excluded.
pub fn fingerprint(spec: &Value) -> String {
    let blocks: Vec<Value> = spec["blocks"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .map(|block| {
                    serde_json::json!([
                        block["family"],
                        block["form"],
                        block.pointer("/vegaLite/data/values").cloned().unwrap_or(Value::Null),
                        block.pointer("/vegaLite/encoding").cloned().unwrap_or(Value::Null),
                        block.get("content").cloned().unwrap_or(Value::Null),
                        block.pointer("/graph/nodes").cloned().unwrap_or(Value::Null),
                        block.pointer("/graph/edges").cloned().unwrap_or(Value::Null),
                    ])
                })
                .collect()
        })
        .unwrap_or_default();
    let digest = Sha256::digest(super::canonical_json(&Value::Array(blocks)).as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

impl RepeatGuard {
    /// Refuse a spec the user has already seen, unless it asks to redraw.
    pub fn check(&self, spec: &Value) -> Result<(), SpecError> {
        if spec.get("redraw") == Some(&Value::Bool(true)) {
            return Ok(());
        }
        if self.recent.contains(&fingerprint(spec)) {
            return Err(SpecError::new(
                "",
                "the user has already seen this exact visual in this conversation: refer back to it in text. Set redraw: true only if the user asked to see it again or to change it",
            ));
        }
        Ok(())
    }

    /// Remember an accepted spec. Rejected specs are never remembered.
    pub fn remember(&mut self, spec: &Value) {
        let print = fingerprint(spec);
        self.recent.retain(|seen| seen != &print);
        self.recent.push_back(print);
        while self.recent.len() > REMEMBERED {
            self.recent.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec(values: Value) -> Value {
        json!({"version": 1, "title": "First", "blocks": [{"family": "chart", "form": "bar", "vegaLite": {
            "mark": "bar", "data": {"values": values}, "encoding": {"x": {"field": "k"}, "y": {"field": "v"}}
        }}]})
    }

    #[test]
    fn the_same_form_and_data_twice_is_rejected_the_second_time() {
        let mut guard = RepeatGuard::default();
        let first = spec(json!([{"k": "a", "v": 1}]));
        assert!(guard.check(&first).is_ok());
        guard.remember(&first);
        assert!(guard.check(&first).is_err());
        assert!(guard.check(&spec(json!([{"k": "a", "v": 2}]))).is_ok());
    }

    #[test]
    fn a_changed_title_does_not_bypass_the_guard() {
        let mut guard = RepeatGuard::default();
        let first = spec(json!([{"k": "a", "v": 1}]));
        guard.remember(&first);
        let mut retitled = first.clone();
        retitled["title"] = json!("Second");
        retitled["followUps"] = json!(["more?"]);
        assert!(guard.check(&retitled).is_err());
    }

    #[test]
    fn key_order_does_not_change_the_fingerprint() {
        let a = spec(json!([{"k": "a", "v": 1}]));
        let b = spec(json!([{"v": 1, "k": "a"}]));
        assert_eq!(fingerprint(&a), fingerprint(&b));
    }

    #[test]
    fn redraw_true_bypasses_the_guard() {
        let mut guard = RepeatGuard::default();
        let first = spec(json!([{"k": "a", "v": 1}]));
        guard.remember(&first);
        let mut again = first.clone();
        again["redraw"] = json!(true);
        assert!(guard.check(&again).is_ok());
    }

    #[test]
    fn the_guard_remembers_only_the_last_10_accepted_calls() {
        let mut guard = RepeatGuard::default();
        let oldest = spec(json!([{"k": "a", "v": 0}]));
        guard.remember(&oldest);
        for at in 1..=REMEMBERED {
            guard.remember(&spec(json!([{"k": "a", "v": at}])));
        }
        assert!(guard.check(&oldest).is_ok());
        assert!(guard.check(&spec(json!([{"k": "a", "v": REMEMBERED}]))).is_err());
    }

    #[test]
    fn a_rejected_call_is_not_remembered() {
        // The server remembers only after validation, lint and this check all
        // pass; a guard that was never told about a spec does not refuse it.
        let guard = RepeatGuard::default();
        assert!(guard.check(&spec(json!([{"k": "a", "v": 1}]))).is_ok());
    }
}

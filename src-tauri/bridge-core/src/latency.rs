//! Where a send's time goes, written down as it happens.
//!
//! The session forest records what a provider said, stamped when Bridge stored
//! it. It cannot say how long the person waited before Bridge launched the
//! provider, what the launch spent its time on, or how long a request took to
//! answer. Those are the hops between pressing Send and the first token that a
//! history read cannot recover, so each one leaves a `latency.*` row in the
//! events ledger with its own breakdown:
//!
//! | kind | written by | breakdown |
//! | --- | --- | --- |
//! | `latency.rpc` | bridged, per send-path request | dispatch, serialize and write time, response bytes |
//! | `latency.rpc_window` | bridged, per minute, for state reads and slow requests | calls, mean and max time, largest response |
//! | `latency.launch` | `start_chat` | resolve, prompt, context, spawn, persist, state |
//! | `latency.submit` | `submit_input` | gates, route, prepare, deliver |
//! | `latency.harness` | the Claude sidecar, per turn | stdin to init, requesting, first byte, first token |
//! | `latency.client` | the webview, per send | click to each RPC, first frame, first output painted |
//!
//! Rows carry durations and names only, never message text. They are what
//! `scripts/latency-report.mjs` turns into percentiles.

use crate::store;
use rusqlite::Connection;
use std::time::Instant;

pub const SOURCE: &str = "latency";

/// Named boundaries one operation crossed, in milliseconds from its start.
pub struct Marks {
    started: Instant,
    steps: Vec<(&'static str, u128)>,
}

impl Marks {
    pub fn start() -> Self {
        Self { started: Instant::now(), steps: Vec::new() }
    }

    pub fn mark(&mut self, name: &'static str) {
        self.steps.push((name, self.started.elapsed().as_millis()));
    }

    pub fn total_ms(&self) -> u128 {
        self.started.elapsed().as_millis()
    }

    /// `{"totalMs": n, "steps": [[name, atMs, tookMs], ...]}`: each step's
    /// own duration beside its offset, so a reader never has to diff them.
    pub fn to_json(&self) -> serde_json::Value {
        let mut previous = 0;
        let steps: Vec<serde_json::Value> = self
            .steps
            .iter()
            .map(|(name, at)| {
                let took = at - previous;
                previous = *at;
                serde_json::json!([name, at, took])
            })
            .collect();
        serde_json::json!({ "totalMs": self.total_ms(), "steps": steps })
    }

    /// One stderr line in the shape the launch timing always printed.
    pub fn line(&self, label: &str) -> String {
        let mut line = format!("bridge: {label} total_ms={}", self.total_ms());
        let mut previous = 0;
        for (name, at) in &self.steps {
            line.push_str(&format!(" {name}_ms={at} (+{})", at - previous));
            previous = *at;
        }
        line
    }
}

/// Append one ledger row. Best-effort: a timing row is never worth failing the
/// operation it describes, so an error is dropped.
pub fn record(db: &Connection, kind: &str, entity_id: &str, body: &serde_json::Value) {
    let _ = store::event(db, SOURCE, kind, entity_id, &body.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_carry_their_own_duration() {
        let mut marks = Marks { started: Instant::now(), steps: vec![("a", 5), ("b", 12)] };
        marks.steps.push(("c", 12));
        let json = marks.to_json();
        assert_eq!(json["steps"], serde_json::json!([["a", 5, 5], ["b", 12, 7], ["c", 12, 0]]));
    }

    #[test]
    fn record_writes_a_latency_row() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE events(id INTEGER PRIMARY KEY, source TEXT, kind TEXT, entity_id TEXT, body TEXT, created_at TEXT)",
        )
        .unwrap();
        record(&db, "latency.rpc", "s1", &serde_json::json!({"method": "state/get_state", "ms": 3}));
        let (source, kind, body): (String, String, String) = db
            .query_row("SELECT source, kind, body FROM events", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap();
        assert_eq!((source.as_str(), kind.as_str()), ("latency", "latency.rpc"));
        assert_eq!(serde_json::from_str::<serde_json::Value>(&body).unwrap()["ms"], 3);
    }
}

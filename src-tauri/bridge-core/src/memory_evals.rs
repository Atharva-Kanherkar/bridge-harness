//! Labeled evals for account memory (issue #885).
//!
//! `packet_eval` needs no model and runs with the rest of the suite: it builds
//! the packet for every labeled ledger in `testing/evals/memory-packet/` and
//! gates what reached it. Its notion of "the same line" is its own (case and
//! whitespace folded), never `memory_similarity`, so the eval cannot agree
//! with a bug in the code it scores.
//!
//! The live evals run real provider turns and are ignored by default:
//!
//! ```text
//! BRIDGE_MEMORY_EVAL=1 cargo test -p bridge-core --lib memory_evals::live_extraction -- --ignored --nocapture
//! BRIDGE_MEMORY_TOOLS_EVAL=1 cargo test -p bridge-core --lib memory_evals::live_tools -- --ignored --nocapture
//! ```
//!
//! `BRIDGE_MEMORY_EVAL_HARNESS` and `BRIDGE_MEMORY_EVAL_MODEL` pick the
//! profile (default `claude` on its default model). Changes to the extraction
//! prompt or to the memory tool descriptions report these numbers in their PR.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{params, Connection};
use serde::Deserialize;

use crate::memory_packet;
use bridge_protocol::messages::ACCOUNT_MEMORY_SCOPE;

const LEDGERS: &str = include_str!("../../../testing/evals/memory-packet/ledgers.json");
/// The longest line the packet may carry, prefix included.
const LINE_CAP: usize = 320;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LedgerFile {
    ledgers: Vec<Ledger>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Ledger {
    name: String,
    records: Vec<LedgerRecord>,
    must_cover: Vec<Vec<String>>,
    duplicate_groups: Vec<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LedgerRecord {
    id: String,
    kind: String,
    provenance: String,
    body: String,
    confidence_bps: Option<i64>,
    conflict_group: Option<String>,
    updated_at: String,
}

#[derive(Debug, Default)]
pub(crate) struct PacketScore {
    pub covered: usize,
    pub must_cover: usize,
    pub constraints_covered: usize,
    pub constraints: usize,
    pub exact_duplicate_lines: usize,
    pub duplicate_groups_repeated: usize,
    pub conflict_groups_repeated: usize,
    pub longest_line: usize,
    pub total_chars: usize,
    pub selected: usize,
}

fn fold(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_string()
}

fn ledger_db(ledger: &Ledger) -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::store::open(&dir.path().join("bridge.db")).unwrap();
    for record in &ledger.records {
        db.execute(
            "INSERT INTO memory_records(id, scope_key, kind, body, provenance, status,
                 confidence_bps, conflict_group, valid_from, created_at, updated_at)
             VALUES(?1,?2,?3,?4,?5,'active',?6,?7,?8,?8,?8)",
            params![
                record.id,
                ACCOUNT_MEMORY_SCOPE,
                record.kind,
                record.body,
                record.provenance,
                record.confidence_bps,
                record.conflict_group,
                record.updated_at,
            ],
        )
        .unwrap();
    }
    (dir, db)
}

fn score(ledger: &Ledger) -> PacketScore {
    let (_dir, db) = ledger_db(ledger);
    let packet = memory_packet::for_session(&db, ACCOUNT_MEMORY_SCOPE, "eval-recipient").unwrap();
    let mut score = PacketScore { must_cover: ledger.must_cover.len(), ..Default::default() };
    let Some(packet) = packet else { return score };
    let selected: BTreeSet<&str> = packet.selected.iter().map(|item| item.record_id.as_str()).collect();
    score.selected = selected.len();
    score.covered = ledger.must_cover.iter().filter(|group| group.iter().any(|id| selected.contains(id.as_str()))).count();

    // A constraint counts as carried when it, or a record labeled as saying
    // the same thing, reached the packet.
    fn same_as<'a>(ledger: &'a Ledger, id: &'a str) -> Vec<&'a str> {
        let mut ids = vec![id];
        for group in &ledger.duplicate_groups {
            if group.iter().any(|member| member == id) {
                ids.extend(group.iter().map(String::as_str));
            }
        }
        ids
    }
    for record in ledger.records.iter().filter(|record| record.kind == "constraint") {
        score.constraints += 1;
        if same_as(ledger, &record.id).iter().any(|id| selected.contains(id)) {
            score.constraints_covered += 1;
        }
    }

    let mut seen = BTreeSet::new();
    for item in &packet.selected {
        if !seen.insert(fold(&item.body)) {
            score.exact_duplicate_lines += 1;
        }
    }
    score.duplicate_groups_repeated = ledger
        .duplicate_groups
        .iter()
        .filter(|group| group.iter().filter(|id| selected.contains(id.as_str())).count() > 1)
        .count();
    let mut per_group: BTreeMap<&str, usize> = BTreeMap::new();
    for record in &ledger.records {
        if let (Some(group), true) = (&record.conflict_group, selected.contains(record.id.as_str())) {
            *per_group.entry(group.as_str()).or_default() += 1;
        }
    }
    score.conflict_groups_repeated = per_group.values().filter(|count| **count > 1).count();

    // The first line is the packet's header; every other line is one record.
    for line in packet.text.lines().skip(1) {
        score.longest_line = score.longest_line.max(line.chars().count());
        score.total_chars += line.len() + 1;
    }
    score
}

fn ledgers() -> Vec<Ledger> {
    serde_json::from_str::<LedgerFile>(LEDGERS).expect("ledgers.json parses").ledgers
}

#[test]
fn packet_eval() {
    let mut failures = Vec::new();
    println!(
        "{:<36} {:>8} {:>11} {:>9} {:>10} {:>8} {:>7} {:>6}",
        "ledger", "covered", "constraints", "dup-lines", "dup-groups", "conflict", "longest", "chars"
    );
    for ledger in ledgers() {
        let score = score(&ledger);
        println!(
            "{:<36} {:>8} {:>11} {:>9} {:>10} {:>8} {:>7} {:>6}",
            ledger.name,
            format!("{}/{}", score.covered, score.must_cover),
            format!("{}/{}", score.constraints_covered, score.constraints),
            score.exact_duplicate_lines,
            score.duplicate_groups_repeated,
            score.conflict_groups_repeated,
            score.longest_line,
            score.total_chars,
        );
        if score.covered != score.must_cover {
            failures.push(format!("{}: covered {}/{}", ledger.name, score.covered, score.must_cover));
        }
        if score.constraints_covered != score.constraints {
            failures.push(format!("{}: constraints {}/{}", ledger.name, score.constraints_covered, score.constraints));
        }
        if score.exact_duplicate_lines != 0 {
            failures.push(format!("{}: {} exact duplicate lines", ledger.name, score.exact_duplicate_lines));
        }
        if score.conflict_groups_repeated != 0 {
            failures.push(format!("{}: a conflict group reached the packet twice", ledger.name));
        }
        if score.longest_line > LINE_CAP {
            failures.push(format!("{}: a {}-char line exceeds {LINE_CAP}", ledger.name, score.longest_line));
        }
        if score.total_chars > memory_packet::MAX_PACKET_CHARS {
            failures.push(format!("{}: {} chars exceeds the packet budget", ledger.name, score.total_chars));
        }
    }
    assert!(failures.is_empty(), "packet eval gates failed:\n{}", failures.join("\n"));
}

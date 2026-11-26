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
//! BRIDGE_MEMORY_EVAL=1 cargo test -p bridge-core --lib memory_evals::live_reconcile -- --ignored --nocapture
//! BRIDGE_MEMORY_TOOLS_EVAL=1 cargo test -p bridge-core --lib memory_evals::live_tools -- --ignored --nocapture
//! BRIDGE_MEMORY_EVAL=1 cargo test -p bridge-core --lib memory_evals::live_dream -- --ignored --nocapture
//! BRIDGE_MEMORY_EVAL=1 BRIDGE_MEMORY_EVAL_SOURCE=path/to/bridge.db cargo test -p bridge-core --lib memory_evals::live_dream_on_a_copy -- --ignored --nocapture
//! ```
//!
//! `live_dream` repeats every labeled dream case `BRIDGE_MEMORY_EVAL_REPEATS`
//! times (default 3) and prints one `DREAM_EVAL_SUMMARY` JSON line.
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

fn score(ledger: &Ledger, retrieval: memory_packet::Retrieval) -> PacketScore {
    let (_dir, db) = ledger_db(ledger);
    let packet = memory_packet::for_session(&db, ACCOUNT_MEMORY_SCOPE, "eval-recipient", retrieval).unwrap();
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
    use memory_packet::Retrieval;
    let mut failures = Vec::new();
    println!(
        "{:<36} {:<13} {:>8} {:>11} {:>9} {:>10} {:>8} {:>7} {:>6}",
        "ledger", "recipient", "covered", "constraints", "dup-lines", "dup-groups", "conflict", "longest", "chars"
    );
    // Both recipients: a chat that can fetch a clipped record's full text, and
    // one (a Cursor chat, a worker) for which the packet is all there is.
    for (ledger, retrieval) in ledgers().into_iter().flat_map(|ledger| {
        let ledger = std::rc::Rc::new(ledger);
        [(ledger.clone(), Retrieval::MemorySearch), (ledger, Retrieval::PacketOnly)]
    }) {
        let score = score(&ledger, retrieval);
        let recipient = if retrieval == Retrieval::MemorySearch { "memory-search" } else { "packet-only" };
        println!(
            "{:<36} {:<13} {:>8} {:>11} {:>9} {:>10} {:>8} {:>7} {:>6}",
            ledger.name,
            recipient,
            format!("{}/{}", score.covered, score.must_cover),
            format!("{}/{}", score.constraints_covered, score.constraints),
            score.exact_duplicate_lines,
            score.duplicate_groups_repeated,
            score.conflict_groups_repeated,
            score.longest_line,
            score.total_chars,
        );
        if score.covered != score.must_cover {
            failures.push(format!("{} ({recipient}): covered {}/{}", ledger.name, score.covered, score.must_cover));
        }
        if score.constraints_covered != score.constraints {
            failures.push(format!("{} ({recipient}): constraints {}/{}", ledger.name, score.constraints_covered, score.constraints));
        }
        if score.exact_duplicate_lines != 0 {
            failures.push(format!("{}: {} exact duplicate lines", ledger.name, score.exact_duplicate_lines));
        }
        if score.conflict_groups_repeated != 0 {
            failures.push(format!("{}: a conflict group reached the packet twice", ledger.name));
        }
        if retrieval == Retrieval::MemorySearch && score.longest_line > LINE_CAP {
            failures.push(format!("{}: a {}-char line exceeds {LINE_CAP}", ledger.name, score.longest_line));
        }
        if score.total_chars > memory_packet::MAX_PACKET_CHARS {
            failures.push(format!("{}: {} chars exceeds the packet budget", ledger.name, score.total_chars));
        }
    }
    assert!(failures.is_empty(), "packet eval gates failed:\n{}", failures.join("\n"));
}

// ---------------------------------------------------------------------------
// Live evals. Real provider turns; ignored by default.

const EXTRACTION_CASES: &str = include_str!("../../../testing/evals/memory-extraction/cases.json");

#[derive(Deserialize)]
struct ExtractionFile {
    cases: Vec<ExtractionCase>,
}

#[derive(Deserialize)]
struct ExtractionCase {
    name: String,
    #[serde(default)]
    pins: Vec<String>,
    messages: Vec<EvalMessage>,
    #[serde(default, rename = "appEvents")]
    app_events: Vec<EvalAppEvent>,
    expect: Vec<Expectation>,
    forbid: Vec<String>,
}

#[derive(Deserialize)]
struct EvalMessage {
    role: String,
    text: String,
}

/// Inserted as `apps::events` writes them. Written directly rather than
/// through `record_tx`, so a case can also hold activity from an app that
/// declares no events, which the digest must leave out.
#[derive(Deserialize)]
struct EvalAppEvent {
    app: String,
    #[serde(rename = "type")]
    event_type: String,
    summary: String,
}

#[derive(Deserialize)]
struct Expectation {
    all: Vec<String>,
}

fn eval_harness() -> String {
    std::env::var("BRIDGE_MEMORY_EVAL_HARNESS").unwrap_or_else(|_| "claude".into())
}

fn eval_model() -> Option<String> {
    std::env::var("BRIDGE_MEMORY_EVAL_MODEL").ok().filter(|model| !model.trim().is_empty())
}

fn eval_registry() -> std::sync::Arc<crate::adapters::AdapterRegistry> {
    let registry = std::sync::Arc::new(crate::adapters::AdapterRegistry::built_in().expect("adapters"));
    // Discovery runs in the background; give it a moment, as the notes eval does.
    std::thread::sleep(std::time::Duration::from_secs(8));
    registry
}

/// The extraction prompt and digest, answered by a real hidden turn.
struct LiveExtraction {
    registry: std::sync::Arc<crate::adapters::AdapterRegistry>,
    scratch: std::path::PathBuf,
}

impl crate::memory_extraction::ExtractionModel for LiveExtraction {
    fn propose(&mut self, instructions: &str, digest: &str) -> Result<crate::memory_extraction::ExtractionOutput, crate::BridgeError> {
        let generation = crate::title_jobs_live::one_hidden_turn(&self.registry, &self.scratch, &eval_harness(), eval_model().as_deref(), instructions, digest, None)
            .map_err(crate::BridgeError::Invalid)?;
        Ok(crate::memory_extraction::ExtractionOutput { text: generation.text, observed_tokens: generation.tokens, spend_microusd: generation.spend_microusd })
    }
}

/// A fresh database holding one case: its pins, its messages, then its app
/// activity.
fn case_db(case: &ExtractionCase) -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::store::open(&dir.path().join("bridge.db")).unwrap();
    db.execute(
        "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind) VALUES('eval',NULL,'claude','eval','idle','reported','direct')",
        [],
    )
    .unwrap();
    for pin in &case.pins {
        crate::memory_ledger::save(&db, pin, None, None).unwrap();
    }
    for (index, message) in case.messages.iter().enumerate() {
        let kind = if message.role == "user" { "user.message" } else { "assistant.message" };
        db.execute(
            "INSERT INTO session_entries(id,session_id,parent_entry_id,sequence,kind,payload,created_at)
             VALUES(?1,'eval',NULL,?2,?3,json_object('text',?4),'now')",
            params![format!("e{}", index + 1), index as i64 + 1, kind, message.text],
        )
        .unwrap();
    }
    for (index, event) in case.app_events.iter().enumerate() {
        db.execute(
            "INSERT INTO session_entries(id,session_id,parent_entry_id,sequence,kind,payload,context_visibility,created_at)
             VALUES(?1,'eval',NULL,?2,'app.event',json_object('appId',?3,'type',?4,'summary',?5,'payload',json_object()),'hidden','now')",
            params![format!("a{}", index + 1), (case.messages.len() + index) as i64 + 1, event.app, event.event_type, event.summary],
        )
        .unwrap();
    }
    (dir, db)
}

/// Offline: every case parses, and activity from an app with no memory scope
/// never reaches the model, so that case cannot pass by luck.
#[test]
fn extraction_cases_reach_the_digest_only_within_scope() {
    let cases = serde_json::from_str::<ExtractionFile>(EXTRACTION_CASES).expect("cases.json parses").cases;
    assert!(cases.iter().any(|case| !case.app_events.is_empty()));
    for case in &cases {
        let (_dir, db) = case_db(case);
        let digest = crate::memory_extraction::build_digest(&db, ACCOUNT_MEMORY_SCOPE, "eval").unwrap().expect("a digest").text;
        for event in &case.app_events {
            let scoped = crate::apps::builtin(&event.app).is_some_and(|app| !app.manifest.memory.is_empty());
            assert_eq!(digest.contains(&event.summary), scoped, "{}: {}", case.name, event.summary);
        }
    }
}

const RECONCILE_CASES: &str = include_str!("../../../testing/evals/memory-extraction/reconcile.json");

#[derive(Deserialize)]
struct ReconcileFile {
    cases: Vec<ReconcileCase>,
}

#[derive(Deserialize)]
struct ReconcileCase {
    name: String,
    known: Vec<KnownRecord>,
    messages: Vec<EvalMessage>,
    expect: ReconcileExpectation,
}

#[derive(Deserialize)]
struct KnownRecord {
    body: String,
    kind: String,
    /// `active`, `pin`, or `proposed`.
    state: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReconcileExpectation {
    #[serde(default)]
    noop: Vec<usize>,
    #[serde(default)]
    update: Vec<usize>,
    #[serde(default)]
    add: Vec<Vec<String>>,
    #[serde(default)]
    pin_untouched: Vec<usize>,
}

/// A fresh database holding one reconcile case, and the ids of its known
/// memories in case order.
fn reconcile_db(case: &ReconcileCase) -> (tempfile::TempDir, Connection, Vec<String>) {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::store::open(&dir.path().join("bridge.db")).unwrap();
    db.execute(
        "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind) VALUES('eval',NULL,'claude','eval','idle','reported','direct')",
        [],
    )
    .unwrap();
    let ids = case
        .known
        .iter()
        .map(|known| match known.state.as_str() {
            "pin" => crate::memory_ledger::save(&db, &known.body, Some(&known.kind), None).unwrap().id,
            state => {
                let id = crate::memory_ledger::insert_proposal(&db, ACCOUNT_MEMORY_SCOPE, &known.body, &known.kind, Some(8_000), None, "eval").unwrap().id;
                if state == "active" {
                    crate::memory_ledger::approve(&db, &id).unwrap();
                }
                id
            }
        })
        .collect();
    for (index, message) in case.messages.iter().enumerate() {
        let kind = if message.role == "user" { "user.message" } else { "assistant.message" };
        db.execute(
            "INSERT INTO session_entries(id,session_id,parent_entry_id,sequence,kind,payload,created_at)
             VALUES(?1,'eval',NULL,?2,?3,json_object('text',?4),'now')",
            params![format!("e{}", index + 1), index as i64 + 1, kind, message.text],
        )
        .unwrap();
    }
    (dir, db, ids)
}

/// Offline: a model can only answer "already known" or "update that one"
/// about a memory it was shown, so every labeled target must reach the digest.
#[test]
fn reconcile_cases_surface_their_target() {
    let cases = serde_json::from_str::<ReconcileFile>(RECONCILE_CASES).expect("reconcile.json parses").cases;
    assert!(cases.len() >= 7, "the labeled set covers every expectation kind");
    for case in &cases {
        let (_dir, db, ids) = reconcile_db(case);
        let digest = crate::memory_extraction::build_digest(&db, ACCOUNT_MEMORY_SCOPE, "eval").unwrap().expect("a digest");
        for index in case.expect.noop.iter().chain(&case.expect.update) {
            assert!(
                digest.known.iter().any(|(_, id)| *id == ids[*index]),
                "{}: '{}' is not among the similar memories:\n{}",
                case.name,
                case.known[*index].body,
                digest.text
            );
        }
    }
}

#[test]
#[ignore = "runs real provider turns"]
fn live_reconcile() {
    assert!(std::env::var_os("BRIDGE_MEMORY_EVAL").is_some(), "set BRIDGE_MEMORY_EVAL=1");
    let cases = serde_json::from_str::<ReconcileFile>(RECONCILE_CASES).expect("reconcile.json parses").cases;
    let mut model = LiveExtraction { registry: eval_registry(), scratch: std::env::temp_dir().join("bridge-memory-eval") };
    let (mut noop_cases, mut duplicates, mut wrong_targets, mut pins_edited, mut spend, mut passed) = (0usize, 0usize, 0usize, 0usize, 0i64, 0usize);
    for case in &cases {
        let (_dir, db, ids) = reconcile_db(case);
        let outcome = crate::memory_extraction::run_extraction(&db, &mut model, ACCOUNT_MEMORY_SCOPE, "eval");
        let (report, error) = match &outcome {
            Ok((report, output, _)) => {
                spend += output.spend_microusd;
                (report.clone(), None)
            }
            Err(error) => (Default::default(), Some(error.to_string())),
        };
        let fresh: Vec<(String, Option<String>)> = db
            .prepare("SELECT lower(body), supersedes FROM memory_records WHERE status='proposed' AND source_session_id='eval' AND id NOT IN (SELECT value FROM json_each(?1))")
            .unwrap()
            .query_map([serde_json::to_string(&ids).unwrap()], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let adds: Vec<&String> = fresh.iter().filter(|(_, supersedes)| supersedes.is_none()).map(|(body, _)| body).collect();
        let updates: Vec<&String> = fresh.iter().filter_map(|(_, supersedes)| supersedes.as_ref()).collect();
        let pins: Vec<&String> = case.known.iter().zip(&ids).filter(|(known, _)| known.state == "pin").map(|(_, id)| id).collect();
        let case_wrong = updates.iter().filter(|target| !case.expect.update.iter().any(|index| ids[*index] == ***target)).count();
        let case_pins = updates.iter().filter(|target| pins.contains(target)).count()
            + case.expect.pin_untouched.iter().filter(|index| {
                db.query_row("SELECT status FROM memory_records WHERE id=?1", [&ids[**index]], |row| row.get::<_, String>(0)).unwrap() != "active"
            }).count();
        let noop_ok = case.expect.noop.is_empty() || (adds.is_empty() && updates.is_empty());
        if !case.expect.noop.is_empty() {
            noop_cases += 1;
            duplicates += usize::from(!adds.is_empty());
        }
        let update_ok = case.expect.update.iter().all(|index| updates.iter().any(|target| **target == ids[*index]));
        let add_ok = case.expect.add.iter().all(|terms| adds.iter().any(|body| terms.iter().all(|term| body.contains(&term.to_lowercase()))));
        let quiet_ok = !(case.expect.noop.is_empty() && case.expect.update.is_empty() && case.expect.add.is_empty() && case.expect.pin_untouched.is_empty())
            || fresh.is_empty();
        wrong_targets += case_wrong;
        pins_edited += case_pins;
        let pass = noop_ok && update_ok && add_ok && quiet_ok && case_wrong == 0 && case_pins == 0 && error.is_none();
        passed += usize::from(pass);
        println!(
            "{} {:<28} known {} updates {} adds {} {}",
            if pass { "PASS" } else { "MISS" },
            case.name,
            report.known,
            updates.len(),
            adds.len(),
            error.map(|error| format!("error: {error}")).unwrap_or_default()
        );
        for (body, supersedes) in &fresh {
            println!("       - {body}{}", if supersedes.is_some() { "  (update)" } else { "" });
        }
    }
    let duplicate_rate = if noop_cases == 0 { 0.0 } else { duplicates as f64 / noop_cases as f64 };
    println!(
        "passed {passed}/{}, duplicate proposals {duplicates}/{noop_cases} ({:.0}%), wrong-target updates {wrong_targets}, pins edited {pins_edited}, spend ${:.4}",
        cases.len(),
        duplicate_rate * 100.0,
        spend as f64 / 1e6
    );
    assert!(duplicate_rate <= 0.10, "duplicate proposal rate {:.0}% is over 10%", duplicate_rate * 100.0);
    assert_eq!(wrong_targets, 0, "an update named the wrong memory");
    assert_eq!(pins_edited, 0, "a pin was edited");
}

#[test]
#[ignore = "runs real provider turns"]
fn live_extraction() {
    assert!(std::env::var_os("BRIDGE_MEMORY_EVAL").is_some(), "set BRIDGE_MEMORY_EVAL=1");
    let cases = serde_json::from_str::<ExtractionFile>(EXTRACTION_CASES).expect("cases.json parses").cases;
    let mut model = LiveExtraction { registry: eval_registry(), scratch: std::env::temp_dir().join("bridge-memory-eval") };
    let (mut gold, mut found, mut proposals, mut correct, mut forbidden, mut spend) = (0usize, 0usize, 0usize, 0usize, 0usize, 0i64);
    for case in &cases {
        let (_dir, db) = case_db(case);
        let outcome = crate::memory_extraction::run_extraction(&db, &mut model, ACCOUNT_MEMORY_SCOPE, "eval");
        let bodies: Vec<String> = db
            .prepare("SELECT body FROM memory_records WHERE status='proposed'")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|body| body.unwrap().to_lowercase())
            .collect();
        if let Ok((_, output, _)) = &outcome {
            spend += output.spend_microusd;
        }
        let matched = case.expect.iter().filter(|expectation| bodies.iter().any(|body| expectation.all.iter().all(|term| body.contains(&term.to_lowercase())))).count();
        let useful = bodies.iter().filter(|body| case.expect.iter().any(|expectation| expectation.all.iter().all(|term| body.contains(&term.to_lowercase())))).count();
        let bad = bodies.iter().filter(|body| case.forbid.iter().any(|phrase| body.contains(&phrase.to_lowercase()))).count();
        gold += case.expect.len();
        found += matched;
        proposals += bodies.len();
        correct += useful;
        forbidden += bad;
        let verdict = if matched == case.expect.len() && bad == 0 && useful == bodies.len() { "PASS" } else { "MISS" };
        println!("{verdict} {:<34} gold {matched}/{} proposals {} forbidden {bad} {}", case.name, case.expect.len(), bodies.len(), outcome.as_ref().err().map(|error| format!("error: {error}")).unwrap_or_default());
        for body in &bodies {
            println!("       - {body}");
        }
    }
    let recall = if gold == 0 { 1.0 } else { found as f64 / gold as f64 };
    let precision = if proposals == 0 { 1.0 } else { correct as f64 / proposals as f64 };
    println!("recall {found}/{gold} ({:.0}%), precision {correct}/{proposals} ({:.0}%), forbidden {forbidden}, spend ${:.4}", recall * 100.0, precision * 100.0, spend as f64 / 1e6);
    assert_eq!(forbidden, 0, "a forbidden proposal reached the review queue");
}

struct ToolsCase {
    ask: &'static str,
    /// A body fragment one agent proposal must contain, or None for "proposes nothing".
    proposal: Option<&'static str>,
    /// Passes when the agent called a Bridge tool whose name contains this, or
    /// its reply contains `or_reply` (an answer read from the packet is a
    /// correct way to say what is remembered).
    calls: Option<&'static str>,
    or_reply: Option<&'static str>,
}

/// A memory tool from any other server. Using one for something later Bridge
/// chats should know is the failure the eval exists to catch: nothing written
/// there reaches Codex or OpenCode chats or the review queue.
fn foreign_memory_tool(name: &str) -> bool {
    let lowered = name.to_lowercase();
    !lowered.starts_with("mcp__bridge__") && ["memory", "remember", "recall"].iter().any(|word| lowered.contains(word))
}

fn tools_cases() -> Vec<ToolsCase> {
    vec![
        ToolsCase { ask: "From now on, never push directly to main. Keep that in mind in every future chat, not just this one.", proposal: Some("main"), calls: Some("memory_propose"), or_reply: None },
        ToolsCase { ask: "What is 17 times 23?", proposal: None, calls: None, or_reply: None },
        // Where the note lands is the notes eval's business; here it must not become memory.
        ToolsCase { ask: "Save a note in my notes titled 'Sync options' that lists yjs and automerge.", proposal: None, calls: None, or_reply: None },
        ToolsCase { ask: "Do you remember anything about how I like my commits written?", proposal: None, calls: Some("memory_search"), or_reply: Some("conventional") },
        ToolsCase { ask: "Lesson for every future security hunt: always check the upstream tracker before deep-diving a candidate. Make sure later chats know this.", proposal: Some("upstream"), calls: Some("memory_propose"), or_reply: None },
        ToolsCase { ask: "Thanks, CI is green now and the PR is merged.", proposal: None, calls: None, or_reply: None },
    ]
}

#[test]
#[ignore = "runs real chats with tools"]
fn live_tools() {
    use crate::adapters::{ShutdownReason, StartRequest, TurnContext};
    use std::io::BufRead;

    assert!(std::env::var_os("BRIDGE_MEMORY_TOOLS_EVAL").is_some(), "set BRIDGE_MEMORY_TOOLS_EVAL=1");
    assert!(
        std::env::var_os(crate::mcp_apps::BIN_ENV).is_some(),
        "point {} at a bridged built from this tree, or the chat gets an old tool list",
        crate::mcp_apps::BIN_ENV
    );
    let dir = tempfile::tempdir().unwrap();
    let core = std::sync::Arc::new(crate::runtime::BridgeCore::for_tests(dir.path()));
    {
        let db = core.db.lock().unwrap();
        db.execute_batch(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/memory-eval','now');
             INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','c','t','main','/tmp/memory-eval/w','ready','now');",
        )
        .unwrap();
        crate::memory_ledger::save(&db, "Uses Conventional Commits and never cites issue numbers in commit messages", Some("constraint"), None).unwrap();
    }
    crate::notes::tool::install(&core);
    crate::notes::tool::serve_listener().expect("the chat-tools listener");
    let registry = eval_registry();
    let cwd = dir.path().join("cwd");
    std::fs::create_dir_all(&cwd).unwrap();
    let only: Option<usize> = std::env::var("BRIDGE_MEMORY_TOOLS_EVAL_ONLY").ok().and_then(|value| value.parse().ok());

    let cases = tools_cases();
    let mut passed = 0;
    let mut foreign_calls = 0;
    for (index, case) in cases.iter().enumerate() {
        if only.is_some_and(|only| only != index) {
            continue;
        }
        let session = format!("eval-chat-{index}");
        // The frame a Bridge launch sends: the capability lines this chat is
        // owed and its memory packet, in the conversation beside the message.
        let frame = {
            let db = core.db.lock().unwrap();
            db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status) VALUES(?1,'w',?2,'eval','ready')", params![session, eval_harness()])
                .unwrap();
            let guidance = crate::memory_tools::session_guidance(&db, &session).unwrap_or_default();
            crate::session_context::build(guidance, crate::memory_packet::for_compile(&db, &session).unwrap().as_deref())
        };
        let cwd_text = cwd.to_string_lossy().to_string();
        let started = registry.start(
            &eval_harness(),
            StartRequest {
                session_id: Some(&session),
                cwd: &cwd_text,
                model: eval_model().as_deref(),
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
        let Ok(started) = started else {
            println!("FAIL {index} could not start a chat");
            continue;
        };
        let mut runtime = started.runtime;
        let context = TurnContext { session: frame.as_ref().map(crate::session_context::SessionContext::text), credentials: None };
        runtime.send_turn_with_context(case.ask, context).unwrap();
        let mut tools: Vec<String> = Vec::new();
        let mut reply = String::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(240);
        let mut lines = started.reader.lines();
        while std::time::Instant::now() < deadline {
            let Some(Ok(line)) = lines.next() else { break };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
            let mut done = false;
            for event in registry.normalize(&eval_harness(), &value) {
                if event.kind == "tool.started" {
                    tools.push(event.title.clone().unwrap_or_default());
                }
                if event.kind == "message.completed" && event.role.as_deref() == Some("assistant") {
                    reply.push_str(event.text.as_deref().unwrap_or_default());
                }
                if event.kind == "turn.completed" || event.kind == "turn.failed" {
                    done = true;
                }
            }
            if done {
                break;
            }
        }
        runtime.stop(ShutdownReason::Completed);

        let proposals: Vec<String> = {
            let db = core.db.lock().unwrap();
            let mut statement = db
                .prepare("SELECT lower(body) FROM memory_records WHERE provenance='agent_proposal' AND source_session_id=?1")
                .unwrap();
            let rows = statement.query_map(params![session], |row| row.get::<_, String>(0)).unwrap();
            rows.map(Result::unwrap).collect()
        };
        let foreign: Vec<&String> = tools.iter().filter(|tool| foreign_memory_tool(tool)).collect();
        foreign_calls += foreign.len();
        let proposal_ok = match case.proposal {
            Some(fragment) => proposals.len() == 1 && proposals[0].contains(fragment),
            None => proposals.is_empty(),
        };
        let calls_ok = case.calls.is_none_or(|name| {
            tools.iter().any(|tool| tool.starts_with("mcp__bridge__") && tool.contains(name))
                || case.or_reply.is_some_and(|phrase| reply.to_lowercase().contains(phrase))
        });
        let ok = proposal_ok && calls_ok && foreign.is_empty();
        passed += usize::from(ok);
        println!("{} {index} {:?}\n      tools {tools:?}\n      proposals {proposals:?}", if ok { "PASS" } else { "FAIL" }, case.ask);
    }
    let total = if only.is_some() { 1 } else { cases.len() };
    println!("memory tools: {passed}/{total}, foreign memory tool calls {foreign_calls}");
    assert_eq!(passed, total);
}

// ---- Nightly dreams (issue #1149) ----

const DREAM_CASES: &str = include_str!("../../../testing/evals/memory-dream/cases.json");

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DreamFile {
    cases: Vec<DreamCase>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DreamCase {
    name: String,
    mode: String,
    now: String,
    records: Vec<DreamCaseRecord>,
    expect: DreamExpect,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DreamCaseRecord {
    body: String,
    kind: String,
    state: String,
    created: String,
    confidence: Option<f64>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct DreamExpect {
    merge: Vec<Vec<usize>>,
    merge_terms: Vec<Vec<String>>,
    stale: Vec<usize>,
    fresh_phrase: Option<String>,
    expire: Vec<usize>,
    approve: Vec<usize>,
    reject: Vec<usize>,
    junk: Vec<usize>,
    keep: Vec<usize>,
    tidy: bool,
}

fn dream_cases() -> Vec<DreamCase> {
    serde_json::from_str::<DreamFile>(DREAM_CASES).expect("memory-dream/cases.json parses").cases
}

fn dream_now(case: &DreamCase) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(&case.now).expect("a case's now parses").with_timezone(&chrono::Utc)
}

/// A fresh database holding one case's records under ids `c0`, `c1`, ….
fn dream_db(case: &DreamCase) -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::store::open(&dir.path().join("bridge.db")).unwrap();
    crate::memory_extraction::update_settings(&db, ACCOUNT_MEMORY_SCOPE, &case.mode).unwrap();
    for (index, record) in case.records.iter().enumerate() {
        let (status, provenance) = match record.state.as_str() {
            "pin" => ("active", "user_explicit"),
            "active" => ("active", "model_proposal"),
            "proposed" => ("proposed", "model_proposal"),
            other => panic!("unknown state {other} in {}", case.name),
        };
        let at = format!("{}T09:00:00+00:00", record.created);
        db.execute(
            "INSERT INTO memory_records(id, scope_key, kind, body, provenance, status, confidence_bps, rationale,
                 valid_from, valid_to, created_at, updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,'Said so in a chat',?8,?9,?8,?8)",
            params![
                format!("c{index}"),
                ACCOUNT_MEMORY_SCOPE,
                record.kind,
                record.body,
                provenance,
                status,
                record.confidence.map(|value| (value * 10_000.0).round() as i64),
                at,
                (status == "proposed").then_some(at.clone()),
            ],
        )
        .unwrap();
    }
    (dir, db)
}

#[derive(Debug, Default, Clone, Copy)]
struct DreamScore {
    pins_changed: usize,
    invented: usize,
    ungrounded: usize,
    merge_sets: usize,
    merges_folded: usize,
    wrong_approvals: usize,
    approvals: usize,
    approvals_hit: usize,
    expiries: usize,
    expiries_hit: usize,
    contradictions: usize,
    contradictions_resolved: usize,
    rejects: usize,
    rejects_hit: usize,
    kept: usize,
    kept_hit: usize,
    tidy: usize,
    tidy_churn: usize,
    applied: usize,
    refused: usize,
}

impl DreamScore {
    fn add(&mut self, other: DreamScore) {
        self.pins_changed += other.pins_changed;
        self.invented += other.invented;
        self.ungrounded += other.ungrounded;
        self.merge_sets += other.merge_sets;
        self.merges_folded += other.merges_folded;
        self.wrong_approvals += other.wrong_approvals;
        self.approvals += other.approvals;
        self.approvals_hit += other.approvals_hit;
        self.expiries += other.expiries;
        self.expiries_hit += other.expiries_hit;
        self.contradictions += other.contradictions;
        self.contradictions_resolved += other.contradictions_resolved;
        self.rejects += other.rejects;
        self.rejects_hit += other.rejects_hit;
        self.kept += other.kept;
        self.kept_hit += other.kept_hit;
        self.tidy += other.tidy;
        self.tidy_churn += other.tidy_churn;
        self.applied += other.applied;
        self.refused += other.refused;
    }

    /// Every case's expectations met, gates included.
    fn perfect(&self) -> bool {
        self.pins_changed == 0
            && self.invented == 0
            && self.wrong_approvals == 0
            && self.merges_folded == self.merge_sets
            && self.approvals_hit == self.approvals
            && self.expiries_hit == self.expiries
            && self.contradictions_resolved == self.contradictions
            && self.rejects_hit == self.rejects
            && self.kept_hit == self.kept
            && self.tidy_churn == 0
    }
}

fn dream_row(db: &Connection, id: &str) -> (String, String, Option<String>) {
    db.query_row("SELECT status, body, expires_at FROM memory_records WHERE id=?1", [id], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    })
    .unwrap()
}

fn live_bodies(db: &Connection) -> Vec<(String, String)> {
    db.prepare("SELECT status, lower(body) FROM memory_records WHERE status IN ('active','proposed')")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// What one dream did to one case, judged by outcome rather than by which
/// operation the model picked: a duplicate set is folded whether it was
/// merged or all but one were rejected.
fn score_dream(case: &DreamCase, db: &Connection, report: &crate::memory_dream::DreamReport) -> DreamScore {
    let id = |index: usize| format!("c{index}");
    let mut score = DreamScore {
        applied: report.applied(),
        refused: report.refused_total() + report.invalid,
        ungrounded: report.refused.get("ungrounded").copied().unwrap_or(0),
        ..DreamScore::default()
    };
    for (index, record) in case.records.iter().enumerate() {
        if record.state == "pin" {
            let (status, body, _) = dream_row(db, &id(index));
            score.pins_changed += usize::from(status != "active" || body != record.body);
        }
    }
    // Invented: a new body with a word no record in the case ever used.
    let vocabulary: Vec<&str> = case.records.iter().map(|record| record.body.as_str()).collect();
    for operation in &report.operations {
        let before: BTreeSet<&str> = operation.before.iter().map(|row| row.id.as_str()).collect();
        for created in operation.after.iter().filter(|row| !before.contains(row.id.as_str())) {
            score.invented += usize::from(!crate::memory_dream::grounded(&created.body, &vocabulary));
        }
    }
    let live = live_bodies(db);
    for (set, terms) in case.expect.merge.iter().zip(&case.expect.merge_terms) {
        score.merge_sets += 1;
        let carrying = live.iter().filter(|(_, body)| terms.iter().all(|term| body.contains(&term.to_lowercase()))).count();
        let members_live = set.iter().filter(|index| matches!(dream_row(db, &id(**index)).0.as_str(), "active" | "proposed")).count();
        score.merges_folded += usize::from(carrying == 1 && members_live <= 1);
    }
    for index in &case.expect.junk {
        score.wrong_approvals += usize::from(dream_row(db, &id(*index)).0 == "active");
    }
    for index in &case.expect.approve {
        score.approvals += 1;
        score.approvals_hit += usize::from(dream_row(db, &id(*index)).0 == "active");
    }
    for index in &case.expect.expire {
        score.expiries += 1;
        let (status, _, expires) = dream_row(db, &id(*index));
        score.expiries_hit += usize::from(status == "expired" || expires.is_some());
    }
    for index in &case.expect.reject {
        score.rejects += 1;
        score.rejects_hit += usize::from(matches!(dream_row(db, &id(*index)).0.as_str(), "rejected" | "superseded"));
    }
    for index in &case.expect.keep {
        score.kept += 1;
        let (status, body, expires) = dream_row(db, &id(*index));
        score.kept_hit += usize::from(status == "active" && body == case.records[*index].body && expires.is_none());
    }
    if let Some(phrase) = &case.expect.fresh_phrase {
        score.contradictions += 1;
        let stale_gone = case.expect.stale.iter().all(|index| dream_row(db, &id(*index)).0 != "active");
        let fresh_active = live.iter().any(|(status, body)| status == "active" && body.contains(phrase.as_str()));
        score.contradictions_resolved += usize::from(stale_gone && fresh_active);
    }
    if case.expect.tidy {
        score.tidy += 1;
        score.tidy_churn += report.applied();
    }
    score
}

/// The answer a careful reader would give a case, in aliases.
fn ideal_answer(case: &DreamCase, plan: &crate::memory_dream::DreamPlan) -> String {
    let alias = |index: usize| plan.alias_of(&format!("c{index}")).expect("every case record is planned").to_string();
    let mut operations: Vec<serde_json::Value> = Vec::new();
    let mut touched: BTreeSet<usize> = BTreeSet::new();
    for set in &case.expect.merge {
        operations.push(serde_json::json!({
            "operation": "merge",
            "targets": set.iter().map(|index| alias(*index)).collect::<Vec<_>>(),
            "body": case.records[set[0]].body,
        }));
        touched.extend(set.iter().copied());
    }
    if case.expect.fresh_phrase.is_some() {
        for stale in &case.expect.stale {
            operations.push(serde_json::json!({ "operation": "retire", "targets": [alias(*stale)] }));
            touched.insert(*stale);
        }
        for (index, record) in case.records.iter().enumerate() {
            if record.state == "proposed" && !touched.contains(&index) {
                operations.push(serde_json::json!({ "operation": "approve", "targets": [alias(index)] }));
                touched.insert(index);
            }
        }
    }
    for index in &case.expect.expire {
        operations.push(serde_json::json!({ "operation": "expire", "targets": [alias(*index)], "afterDays": 1 }));
        touched.insert(*index);
    }
    for index in &case.expect.approve {
        operations.push(serde_json::json!({ "operation": "approve", "targets": [alias(*index)] }));
        touched.insert(*index);
    }
    for index in &case.expect.reject {
        let mut reject = serde_json::json!({ "operation": "reject", "targets": [alias(*index)], "reason": "Not worth keeping" });
        if let Some(original) = (0..case.records.len()).find(|other| *other != *index && case.records[*other].state != "proposed") {
            reject["duplicateOf"] = alias(original).into();
        }
        operations.push(reject);
        touched.insert(*index);
    }
    for index in (0..case.records.len()).filter(|index| !touched.contains(index)) {
        operations.push(serde_json::json!({ "operation": "keep", "targets": [alias(index)] }));
    }
    format!("```bridge-memory-operations\n{}\n```", serde_json::Value::Array(operations))
}

struct CannedDream(String);

impl crate::memory_consolidation::ConsolidationModel for CannedDream {
    fn consolidate(&mut self, _prompt: &str) -> Result<crate::memory_consolidation::ConsolidationOutput, crate::BridgeError> {
        Ok(crate::memory_consolidation::ConsolidationOutput { text: self.0.clone(), observed_tokens: 0, spend_microusd: 0 })
    }
}

#[test]
fn dream_cases_hold_offline() {
    let cases = dream_cases();
    assert!(cases.len() >= 10, "every labeled case from the contract is here");
    for case in &cases {
        let (_dir, db) = dream_db(case);
        let plan = crate::memory_dream::plan(&db, ACCOUNT_MEMORY_SCOPE).unwrap();
        assert_eq!(plan.batches.len(), 1, "{} fits one batch", case.name);
        let mut model = CannedDream(ideal_answer(case, &plan));
        let (report, _) = crate::memory_dream::run_dream(&db, &mut model, ACCOUNT_MEMORY_SCOPE, dream_now(case)).unwrap();
        let score = score_dream(case, &db, &report);
        assert!(score.perfect(), "{}: an ideal answer must meet every expectation: {score:?} {report:?}", case.name);
    }

    // Adversarial answers the gate must refuse, each changing nothing.
    let pin_case = cases.iter().find(|case| case.name == "pin-with-paraphrase").unwrap();
    let (_dir, db) = dream_db(pin_case);
    let plan = crate::memory_dream::plan(&db, ACCOUNT_MEMORY_SCOPE).unwrap();
    let (pin, copy) = (plan.alias_of("c0").unwrap(), plan.alias_of("c1").unwrap());
    let mut model = CannedDream(format!(
        "```bridge-memory-operations\n[{{\"operation\":\"correct\",\"targets\":[\"{pin}\"],\"basis\":\"{copy}\",\"body\":\"Never push directly to the main branch\"}},\
         {{\"operation\":\"retire\",\"targets\":[\"c0\"]}},{{\"operation\":\"approve\",\"targets\":[\"{copy}\"]}}]\n```"
    ));
    let (report, _) = crate::memory_dream::run_dream(&db, &mut model, ACCOUNT_MEMORY_SCOPE, dream_now(pin_case)).unwrap();
    assert_eq!(report.refused.get("locked"), Some(&1), "a pin edit: {report:?}");
    assert_eq!(report.refused.get("foreign_id"), Some(&1), "a raw id");
    assert_eq!(report.refused.get("conflict"), Some(&1), "approving a repeat of a pin");
    assert_eq!(score_dream(pin_case, &db, &report).pins_changed, 0);

    let researcher = cases.iter().find(|case| case.name == "researcher-variants").unwrap();
    let (_dir, db) = dream_db(researcher);
    let plan = crate::memory_dream::plan(&db, ACCOUNT_MEMORY_SCOPE).unwrap();
    let (a, b) = (plan.alias_of("c0").unwrap(), plan.alias_of("c1").unwrap());
    let mut model = CannedDream(format!(
        "```bridge-memory-operations\n[{{\"operation\":\"merge\",\"targets\":[\"{a}\",\"{b}\"],\"body\":\"Works as an independent researcher in Berlin\"}}]\n```"
    ));
    let (report, _) = crate::memory_dream::run_dream(&db, &mut model, ACCOUNT_MEMORY_SCOPE, dream_now(researcher)).unwrap();
    assert_eq!(report.refused.get("ungrounded"), Some(&1), "an ungrounded merge");
    assert_eq!(report.applied(), 0);

    let junk = cases.iter().find(|case| case.name == "junk-status-updates").unwrap();
    let (_dir, db) = dream_db(junk);
    let plan = crate::memory_dream::plan(&db, ACCOUNT_MEMORY_SCOPE).unwrap();
    let (first, second) = (plan.alias_of("c0").unwrap(), plan.alias_of("c1").unwrap());
    let mut model = CannedDream(format!(
        "```bridge-memory-operations\n[{{\"operation\":\"approve\",\"targets\":[\"{first}\"]}},{{\"operation\":\"approve\",\"targets\":[\"{second}\"]}}]\n```"
    ));
    let (report, _) = crate::memory_dream::run_dream(&db, &mut model, ACCOUNT_MEMORY_SCOPE, dream_now(junk)).unwrap();
    assert_eq!(score_dream(junk, &db, &report).wrong_approvals, 0, "approving junk: {report:?}");
}

/// One hidden bounded turn per batch on the eval profile, as the host runs a
/// dream: an empty tool scope and one turn.
struct LiveDream {
    registry: std::sync::Arc<crate::adapters::AdapterRegistry>,
    scratch: std::path::PathBuf,
}

impl crate::memory_consolidation::ConsolidationModel for LiveDream {
    fn consolidate(&mut self, prompt: &str) -> Result<crate::memory_consolidation::ConsolidationOutput, crate::BridgeError> {
        let generation = crate::title_jobs_live::one_hidden_turn(&self.registry, &self.scratch, &eval_harness(), eval_model().as_deref(), "", prompt, None)
            .map_err(crate::BridgeError::Invalid)?;
        Ok(crate::memory_consolidation::ConsolidationOutput {
            text: generation.text,
            observed_tokens: generation.tokens,
            spend_microusd: generation.spend_microusd,
        })
    }
}

#[test]
#[ignore = "runs real provider turns"]
fn live_dream() {
    assert!(std::env::var_os("BRIDGE_MEMORY_EVAL").is_some(), "set BRIDGE_MEMORY_EVAL=1");
    let repeats: usize = std::env::var("BRIDGE_MEMORY_EVAL_REPEATS").ok().and_then(|value| value.parse().ok()).unwrap_or(3);
    let cases = dream_cases();
    let mut model = LiveDream { registry: eval_registry(), scratch: std::env::temp_dir().join("bridge-memory-dream-eval") };
    let (mut total, mut spend, mut wall, mut perfect, mut failures) = (DreamScore::default(), 0i64, 0f64, 0usize, 0usize);
    for round in 1..=repeats {
        for case in &cases {
            let (_dir, db) = dream_db(case);
            let started = std::time::Instant::now();
            let outcome = crate::memory_dream::run_dream(&db, &mut model, ACCOUNT_MEMORY_SCOPE, dream_now(case));
            wall += started.elapsed().as_secs_f64();
            let (report, output) = match outcome {
                Ok(done) => done,
                Err(error) => {
                    failures += 1;
                    println!("ERROR round {round} {:<30} {error}", case.name);
                    continue;
                }
            };
            spend += output.spend_microusd;
            let score = score_dream(case, &db, &report);
            perfect += usize::from(score.perfect());
            total.add(score);
            let operations: Vec<String> = report.operations.iter().map(|operation| operation.operation.clone()).collect();
            println!(
                "{} round {round} {:<30} ops {:?} kept {} refused {:?}",
                if score.perfect() { "PASS" } else { "MISS" },
                case.name,
                operations,
                report.kept,
                report.refused
            );
            if !score.perfect() {
                for (status, body) in live_bodies(&db) {
                    println!("       {status:<8} {body}");
                }
            }
        }
    }
    let runs = repeats * cases.len();
    let ratio = |hit: usize, of: usize| if of == 0 { 1.0 } else { hit as f64 / of as f64 };
    let merge_recall = ratio(total.merges_folded, total.merge_sets);
    let summary = serde_json::json!({
        "model": eval_model().unwrap_or_else(|| "default".into()),
        "runs": runs,
        "perfect": perfect,
        "failures": failures,
        "pinsChanged": total.pins_changed,
        "inventedLanded": total.invented,
        "ungroundedRefused": total.ungrounded,
        "mergeRecall": merge_recall,
        "wrongApprovals": total.wrong_approvals,
        "approvalRecall": ratio(total.approvals_hit, total.approvals),
        "expiryHit": ratio(total.expiries_hit, total.expiries),
        "contradictionsResolved": ratio(total.contradictions_resolved, total.contradictions),
        "rejectHit": ratio(total.rejects_hit, total.rejects),
        "keptIntact": ratio(total.kept_hit, total.kept),
        "tidyChurn": total.tidy_churn,
        "applied": total.applied,
        "refused": total.refused,
        "spendUsd": spend as f64 / 1e6,
        "wallSecondsPerDream": wall / runs as f64,
    });
    println!("DREAM_EVAL_SUMMARY {summary}");
    assert_eq!(failures, 0, "a dream failed outright");
    assert_eq!(total.pins_changed, 0, "a pin was changed");
    assert_eq!(total.invented, 0, "a fact nobody wrote landed");
    assert!(merge_recall >= 0.8, "merge recall {merge_recall:.2} is under 0.80");
    assert_eq!(total.wrong_approvals, 0, "labeled junk was approved");
}

/// One full dream over a copy of a real database's memory, reporting
/// aggregates only.
///
/// `BRIDGE_MEMORY_EVAL_SOURCE` names a real `bridge.db`. It is attached
/// read-only and only its memory tables are copied into a fresh database in a
/// temp dir, which is the one the dream changes. Per-change detail, which
/// holds memory text, goes only to `BRIDGE_MEMORY_EVAL_DETAIL` when set.
#[test]
#[ignore = "runs real provider turns over a copy of real data"]
fn live_dream_on_a_copy() {
    assert!(std::env::var_os("BRIDGE_MEMORY_EVAL").is_some(), "set BRIDGE_MEMORY_EVAL=1");
    let source = std::env::var("BRIDGE_MEMORY_EVAL_SOURCE").expect("set BRIDGE_MEMORY_EVAL_SOURCE to a bridge.db");
    let dir = tempfile::tempdir().unwrap();
    let db = crate::store::open(&dir.path().join("bridge.db")).unwrap();
    // Read-only by URI, so nothing here can write to the real database.
    db.execute("ATTACH DATABASE ?1 AS source", [format!("file:{}?mode=ro", source.replace(' ', "%20"))]).unwrap();
    for table in ["memory_records", "memory_extraction_settings", "memory_consolidation_settings", "memory_retention_settings"] {
        let columns: Vec<String> = db
            .prepare(&format!("SELECT name FROM pragma_table_info('{table}') WHERE name IN (SELECT name FROM source.pragma_table_info('{table}'))"))
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        if columns.is_empty() {
            continue;
        }
        let list = columns.join(", ");
        db.execute_batch(&format!("DELETE FROM main.{table}; INSERT INTO main.{table}({list}) SELECT {list} FROM source.{table};"))
            .unwrap();
    }
    db.execute_batch("DETACH DATABASE source").unwrap();
    let mut model = LiveDream { registry: eval_registry(), scratch: std::env::temp_dir().join("bridge-memory-dream-eval") };
    let before: BTreeMap<String, i64> = db
        .prepare("SELECT status || '/' || provenance, COUNT(*) FROM memory_records WHERE status IN ('active','proposed') GROUP BY 1")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let started = std::time::Instant::now();
    let (report, output) = crate::memory_dream::run_dream(&db, &mut model, ACCOUNT_MEMORY_SCOPE, chrono::Utc::now()).unwrap();
    let tally = report.tally();
    let summary = serde_json::json!({
        "model": eval_model().unwrap_or_else(|| "default".into()),
        "before": before,
        "batches": report.batches,
        "failedBatches": report.failed_batches,
        "deferred": report.deferred,
        "applied": { "merged": tally.merged, "corrected": tally.corrected, "expired": tally.expired, "grouped": tally.grouped,
                     "retired": tally.retired, "approved": tally.approved, "rejected": tally.rejected },
        "kept": report.kept,
        "refused": report.refused,
        "invalid": report.invalid,
        "spendUsd": output.spend_microusd as f64 / 1e6,
        "wallSeconds": started.elapsed().as_secs_f64(),
    });
    println!("DREAM_COPY_SUMMARY {summary}");
    if let Ok(detail) = std::env::var("BRIDGE_MEMORY_EVAL_DETAIL") {
        std::fs::write(&detail, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    }
    assert_eq!(crate::memory_dream::dream_records(&db, ACCOUNT_MEMORY_SCOPE).unwrap().iter().filter(|entry| entry.record.provenance == "user_explicit").count() as i64,
        before.get("active/user_explicit").copied().unwrap_or(0), "every pin is still active");
}

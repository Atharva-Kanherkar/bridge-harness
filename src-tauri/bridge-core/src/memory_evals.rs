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
    expect: Vec<Expectation>,
    forbid: Vec<String>,
}

#[derive(Deserialize)]
struct EvalMessage {
    role: String,
    text: String,
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

#[test]
#[ignore = "runs real provider turns"]
fn live_extraction() {
    assert!(std::env::var_os("BRIDGE_MEMORY_EVAL").is_some(), "set BRIDGE_MEMORY_EVAL=1");
    let cases = serde_json::from_str::<ExtractionFile>(EXTRACTION_CASES).expect("cases.json parses").cases;
    let mut model = LiveExtraction { registry: eval_registry(), scratch: std::env::temp_dir().join("bridge-memory-eval") };
    let (mut gold, mut found, mut proposals, mut correct, mut forbidden, mut spend) = (0usize, 0usize, 0usize, 0usize, 0usize, 0i64);
    for case in &cases {
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

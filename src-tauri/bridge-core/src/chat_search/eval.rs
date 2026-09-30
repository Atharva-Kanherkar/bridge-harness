//! The search funnel measured against a labelled corpus.
//!
//! `testing/fixtures/chat_search/corpus.json` holds 200 chats (40 labelled
//! targets, 10 same-topic twins at another time, 150 background chats built
//! from overlapping engineering words) and 40 queries in three classes:
//! exact keywords, vague topics, and a topic with a time phrase. Every chat is
//! padded with shared filler to 100 entries, so the index holds 20,000.
//!
//! `index_recall_on_the_labelled_corpus` runs in `cargo test` and asserts the
//! index-only numbers. The model stage needs a provider, so CI measures what
//! it would be sent rather than what it would answer. `bench` (ignored) scales
//! the corpus to 100k entries for latency and the three baselines.

use std::time::Instant;

use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection};
use serde::Deserialize;

use super::agent::{first_turn, SearchInput, INSTRUCTIONS};
use super::parse::{parse, rank_terms_counted};
use super::retrieve::retrieve;
use super::tools::{estimate_tokens, Shown};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Corpus {
    now: String,
    entries_per_chat: usize,
    filler: Vec<String>,
    chats: Vec<Chat>,
    queries: Vec<Query>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Chat {
    id: String,
    title: String,
    harness: String,
    days_ago: i64,
    keys: Vec<String>,
}

#[derive(Deserialize)]
struct Query {
    query: String,
    expect: String,
    class: String,
}

fn corpus() -> Corpus {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testing/fixtures/chat_search/corpus.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn now(corpus: &Corpus) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(&corpus.now).unwrap().with_timezone(&Utc)
}

/// Load `copies` copies of the corpus. Copy 0 keeps the labelled ids; later
/// copies are suffixed background noise that only makes the index bigger.
fn load(corpus: &Corpus, copies: usize) -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut db = crate::store::open(&dir.path().join("bridge.db")).unwrap();
    let now = now(corpus);
    let transaction = db.transaction().unwrap();
    // A small deterministic generator so filler placement is stable.
    let mut state: u64 = 762;
    let mut next = || {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (state >> 33) as usize
    };
    for copy in 0..copies {
        for chat in &corpus.chats {
            let id = if copy == 0 { chat.id.clone() } else { format!("{}-copy{copy}", chat.id) };
            let title = if copy == 0 {
                chat.title.clone()
            } else {
                // Copies carry background words only, so they compete for
                // common terms without duplicating any target's topic.
                format!("Copy {copy} of routine work")
            };
            let started = now - Duration::days(chat.days_ago) - Duration::hours(2);
            transaction
                .execute(
                    "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind,title,started_at)
                     VALUES(?1,NULL,?2,'Chat','idle','reported','direct',?3,?4)",
                    params![id, chat.harness, title, started.to_rfc3339()],
                )
                .unwrap();
            let keys: Vec<&String> = if copy == 0 { chat.keys.iter().collect() } else { Vec::new() };
            let mut insert = transaction
                .prepare_cached(
                    "INSERT INTO session_entries(id,session_id,sequence,kind,payload,context_visibility,created_at)
                     VALUES(?1,?2,?3,?4,?5,'eligible',?6)",
                )
                .unwrap();
            for sequence in 0..corpus.entries_per_chat {
                // Keys land on the first message and then spread through the
                // chat, the way a topic recurs.
                let text = match sequence {
                    0 => keys.first().map(|key| key.to_string()),
                    sequence if sequence % 30 == 7 => keys.get(sequence / 30 + 1).map(|key| key.to_string()),
                    _ => None,
                }
                .unwrap_or_else(|| corpus.filler[next() % corpus.filler.len()].clone());
                let kind = if sequence % 2 == 0 { "user.message" } else { "assistant.message" };
                let at = started + Duration::seconds(sequence as i64 * 60);
                insert
                    .execute(params![
                        format!("{id}-{sequence}"),
                        id,
                        sequence as i64 + 1,
                        kind,
                        serde_json::json!({ "text": text }).to_string(),
                        at.to_rfc3339()
                    ])
                    .unwrap();
            }
        }
    }
    transaction.commit().unwrap();
    (dir, db)
}

struct Measured {
    query: String,
    expect: String,
    top: Option<String>,
    class: String,
    found: bool,
    confident: bool,
    micros: u128,
    t2_first_turn_tokens: usize,
}

fn measure(db: &Connection, corpus: &Corpus) -> Vec<Measured> {
    let now = now(corpus);
    corpus
        .queries
        .iter()
        .map(|query| {
            let started = Instant::now();
            let parsed = parse(&query.query, now);
            let (terms, unknown) = rank_terms_counted(db, &parsed).unwrap();
            let mut found = retrieve(db, &parsed, &terms, 8, now).unwrap();
            found.unknown_terms = unknown;
            let micros = started.elapsed().as_micros();
            let hit = found.candidates.iter().take(4).any(|candidate| candidate.session_id == query.expect);
            let words: Vec<String> = terms.iter().map(|term| term.text.clone()).collect();
            let input = SearchInput { query: &query.query, parsed: &parsed, terms: &words, seed: &found.candidates };
            let prompt = first_turn(db, &mut Shown::default(), &input);
            Measured {
                query: query.query.clone(),
                expect: query.expect.clone(),
                top: found.candidates.first().map(|candidate| candidate.session_id.clone()),
                class: query.class.clone(),
                found: hit,
                confident: found.confident(),
                micros,
                t2_first_turn_tokens: estimate_tokens(INSTRUCTIONS) + estimate_tokens(&prompt),
            }
        })
        .collect()
}

impl Measured {
    fn top_is_expected(&self) -> bool {
        self.top.as_deref() == Some(self.expect.as_str())
    }
}

fn percentile(values: &mut [u128], fraction: f64) -> u128 {
    values.sort_unstable();
    let index = ((values.len() as f64 - 1.0) * fraction).round() as usize;
    values[index]
}

fn recall(results: &[Measured], class: Option<&str>) -> (usize, usize) {
    let scoped: Vec<_> = results.iter().filter(|result| class.is_none_or(|class| result.class == class)).collect();
    (scoped.iter().filter(|result| result.found).count(), scoped.len())
}

fn report(results: &[Measured]) -> String {
    let mut out = String::from("| class | recall@4 (index only) | resolved without the model |\n|---|---|---|\n");
    let resolved = |class: Option<&str>| {
        results
            .iter()
            .filter(|result| class.is_none_or(|class| result.class == class) && result.confident)
            .count()
    };
    for class in ["exact", "vague", "time"] {
        let (found, total) = recall(results, Some(class));
        out.push_str(&format!("| {class} | {found}/{total} | {}/{total} |\n", resolved(Some(class))));
    }
    let (found, total) = recall(results, None);
    out.push_str(&format!("| all | {found}/{total} | {}/{total} |\n", resolved(None)));
    let confident = results.iter().filter(|result| result.confident).count();
    let confident_wrong = results.iter().filter(|result| result.confident && !result.top_is_expected()).count();
    out.push_str(&format!("\nresolved by the index alone (gate confident): {confident}/{}\n", results.len()));
    out.push_str(&format!("confident but top hit wrong: {confident_wrong}\n"));
    for result in results.iter().filter(|result| !result.found || !result.confident) {
        let label = if result.found { "unsure" } else { "miss" };
        out.push_str(&format!("  {label} [{}] {:?} top={:?} confident={}\n", result.class, result.query, result.top, result.confident));
    }
    let mut micros: Vec<u128> = results.iter().map(|result| result.micros).collect();
    out.push_str(&format!(
        "index latency: p50 {:.1} ms, p95 {:.1} ms\n",
        percentile(&mut micros, 0.5) as f64 / 1000.0,
        percentile(&mut micros, 0.95) as f64 / 1000.0
    ));
    let mut prompts: Vec<u128> = results.iter().filter(|result| !result.confident).map(|result| result.t2_first_turn_tokens as u128).collect();
    if !prompts.is_empty() {
        out.push_str(&format!(
            "model first-turn input for unsure queries: median {} tokens, max {} tokens\n",
            percentile(&mut prompts, 0.5),
            prompts.iter().max().unwrap()
        ));
    }
    out
}

#[test]
fn index_recall_on_the_labelled_corpus() {
    let corpus = corpus();
    assert_eq!(corpus.chats.len(), 200);
    assert_eq!(corpus.queries.len(), 40);
    let (_dir, db) = load(&corpus, 1);
    let entries: i64 = db.query_row("SELECT count(*) FROM session_entry_fts", [], |row| row.get(0)).unwrap();
    assert_eq!(entries, 20_000);
    let results = measure(&db, &corpus);
    println!("{}", report(&results));

    // Exact keywords and time-scoped topics are the index's job.
    let (exact, exact_total) = recall(&results, Some("exact"));
    let (time, time_total) = recall(&results, Some("time"));
    assert!(exact * 10 >= exact_total * 9, "exact recall@4 {exact}/{exact_total}\n{}", report(&results));
    assert!(time * 10 >= time_total * 9, "time recall@4 {time}/{time_total}\n{}", report(&results));
    // Vague queries are where words the user remembers were never typed; the
    // index's share there is the bar the model stage and any future
    // embeddings are measured against, so it is pinned rather than hoped for.
    let (vague, _) = recall(&results, Some("vague"));
    assert!(vague >= VAGUE_INDEX_FLOOR, "vague recall@4 regressed to {vague}\n{}", report(&results));
    // The gate may send a clear query to the model, but it must never keep
    // a wrong answer from it: Enter opens a confident top hit directly.
    let confident_wrong = results.iter().filter(|result| result.confident && !result.top_is_expected()).count();
    assert_eq!(confident_wrong, 0, "{}", report(&results));
    // A first model turn stays within the budget the issue set.
    for result in results.iter().filter(|result| !result.confident) {
        assert!(result.t2_first_turn_tokens <= 1_500, "{} tokens", result.t2_first_turn_tokens);
    }
}

/// The measured index-only floor for the vague class; see the PR report.
const VAGUE_INDEX_FLOOR: usize = 6;

/// `cargo test -p bridge-core --lib chat_search::eval::bench -- --ignored --nocapture`
#[test]
#[ignore]
fn bench() {
    let corpus = corpus();
    let (_dir, db) = load(&corpus, 5);
    let entries: i64 = db.query_row("SELECT count(*) FROM session_entry_fts", [], |row| row.get(0)).unwrap();
    println!("corpus: {} chats, {entries} entries", corpus.chats.len() * 5);
    let results = measure(&db, &corpus);
    println!("{}", report(&results));

    // (a) Titles only: every title in one prompt.
    let titles: String = db
        .prepare("SELECT coalesce(title,'') FROM sessions")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>()
        .join("\n");
    println!("baseline (a) titles-only prompt: {} tokens per query", estimate_tokens(&titles));

    // (b) Per-session recall over every chat, one query each.
    let ids: Vec<String> = db
        .prepare("SELECT id FROM sessions")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let started = Instant::now();
    for query in corpus.queries.iter().take(10) {
        for id in &ids {
            let _ = crate::session_recall::search(&db, id, &query.query, Some(4));
        }
    }
    println!(
        "baseline (b) per-session search_session_entries over every chat: {:.1} ms per query",
        started.elapsed().as_secs_f64() * 1000.0 / 10.0
    );

    // (c) Every digest in one prompt.
    let digests: String = db
        .prepare("SELECT body FROM chat_digests")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>()
        .join("\n");
    println!("baseline (c) all summaries prompt: {} tokens per query", estimate_tokens(&digests));
    let raw: i64 = db
        .query_row("SELECT sum(length(body)) FROM session_entry_fts", [], |row| row.get(0))
        .unwrap();
    println!("baseline (d) every entry to a model: {} tokens per query", raw / 4);
    let digest_bytes: i64 = db.query_row("SELECT sum(length(body)) FROM chat_digests", [], |row| row.get(0)).unwrap();
    println!("digest index size: {} bytes over {} chats", digest_bytes, ids.len());
}

/// One live deep search per query against a copy of a real database, on the
/// selected live harness. Never run against the live file: the store migrates
/// what it opens.
///
/// `BRIDGE_CHAT_SEARCH_LIVE_DB=/tmp/copy/bridge.db cargo test -p bridge-core --lib
/// chat_search::eval::live -- --ignored --nocapture`, with queries in
/// `BRIDGE_CHAT_SEARCH_LIVE_QUERIES` separated by `|`.
///
/// It builds the core without `BridgeCore::boot` on purpose: boot reaps the
/// adapter processes a database records, and a copy's records name the live
/// app's processes.
#[test]
#[ignore]
fn live() {
    use bridge_protocol::messages::SearchChatsParams;
    let Ok(path) = std::env::var("BRIDGE_CHAT_SEARCH_LIVE_DB") else {
        println!("set BRIDGE_CHAT_SEARCH_LIVE_DB");
        return;
    };
    let scratch = tempfile::tempdir().unwrap();
    let mut core = crate::runtime::BridgeCore::for_tests(scratch.path());
    let opened = Instant::now();
    core.db = std::sync::Mutex::new(crate::store::open(std::path::Path::new(&path)).unwrap());
    println!("open + migrate: {:.1} s", opened.elapsed().as_secs_f64());
    let harness = std::env::var("BRIDGE_CHAT_SEARCH_LIVE_HARNESS").unwrap_or_else(|_| "claude".into());
    assert!(matches!(harness.as_str(), "claude" | "codex"), "live harness must be claude or codex");
    if harness == "claude" {
        core.adapter_registry = std::sync::Arc::new(crate::adapters::AdapterRegistry::claude_only().unwrap());
    }
    let core = std::sync::Arc::new(core);
    let waited = Instant::now();
    while harness == "claude" && !core
        .adapter_registry
        .descriptors()
        .iter()
        .any(|descriptor| descriptor.id == "claude" && descriptor.available)
    {
        assert!(waited.elapsed().as_secs() < 60, "claude never became available");
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    let queries = std::env::var("BRIDGE_CHAT_SEARCH_LIVE_QUERIES").unwrap_or_else(|_| "plugins catalog stall".into());
    let model = std::env::var("BRIDGE_CHAT_SEARCH_LIVE_MODEL").unwrap_or_else(|_| super::eval_codex::DEFAULT_MODEL.into());
    // Claude goes through the production entry point and its fixed budget.
    let budget = if harness == "codex" { live_budget() } else { super::agent::Budget::default() };
    println!("live harness={harness} wall_budget={}s (production=8s)", budget.wall.as_secs());
    for query in queries.split('|') {
        for deep in [false, true] {
            let params = SearchChatsParams { query: query.into(), limit: None, deep };
            let result = if harness == "codex" {
                super::search_with_budget(&core.db, &params, Utc::now(), super::DeepGate::Allowed, &budget, || {
                    super::eval_codex::CodexEvalModel::start(&core, &model)
                        .map(|model| Box::new(model) as Box<dyn super::agent::SearchModel>)
                })
            } else {
                super::search(&core, &params)
            }.unwrap();
            println!(
                "{query:?} deep={deep} stage={:?} confident={} ms={} model_tokens={} tool_calls={} detail={:?}",
                result.stage, result.confident, result.elapsed_ms, result.model_tokens, result.tool_calls, result.detail
            );
            for hit in &result.hits {
                println!("    {} | {} | {}", &hit.session_id[..8.min(hit.session_id.len())], hit.title, hit.why);
            }
            if result.confident {
                break;
            }
        }
    }
    let db = core.db.lock().unwrap();
    let (rows, turns): (i64, i64) = db
        .query_row(
            "SELECT count(*), count(DISTINCT session_id) FROM usage_ledger WHERE task_family='chat_search'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    println!("usage_ledger rows with task_family=chat_search: {rows} across {turns} hidden sessions");
    let visible: i64 = db
        .query_row("SELECT count(*) FROM sessions WHERE kind='chat_search' AND ended_at IS NULL", [], |row| row.get(0))
        .unwrap();
    println!("unsettled chat_search sessions: {visible}");
}

/// The whole funnel on the labelled corpus with the model stage live on
/// the selected harness: the numbers CI cannot produce. Synthetic chats only, so nothing
/// real is sent to a provider.
///
/// `cargo test -p bridge-core --lib chat_search::eval::live_fixture -- --ignored --nocapture`
#[test]
#[ignore]
fn live_fixture() {
    use bridge_protocol::messages::{ChatSearchStage, SearchChatsParams};
    let corpus = corpus();
    let (_dir, db) = load(&corpus, 1);
    let scratch = tempfile::tempdir().unwrap();
    let mut core = crate::runtime::BridgeCore::for_tests(scratch.path());
    core.db = std::sync::Mutex::new(db);
    let harness = std::env::var("BRIDGE_CHAT_SEARCH_LIVE_HARNESS").unwrap_or_else(|_| "claude".into());
    assert!(matches!(harness.as_str(), "claude" | "codex"), "live harness must be claude or codex");
    if harness == "claude" {
        core.adapter_registry = std::sync::Arc::new(crate::adapters::AdapterRegistry::claude_only().unwrap());
    }
    let core = std::sync::Arc::new(core);
    let waited = Instant::now();
    while harness == "claude" && !core
        .adapter_registry
        .descriptors()
        .iter()
        .any(|descriptor| descriptor.id == "claude" && descriptor.available)
    {
        assert!(waited.elapsed().as_secs() < 60, "claude never became available");
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    let model = std::env::var("BRIDGE_CHAT_SEARCH_LIVE_MODEL").unwrap_or_else(|_| {
        if harness == "codex" { super::eval_codex::DEFAULT_MODEL.into() } else { super::settings::DEFAULT_MODEL.into() }
    });
    let budget = live_budget();
    println!("live harness={harness} model={model} wall_budget={}s (production=8s)", budget.wall.as_secs());
    if harness == "codex" {
        println!("evaluation-only codex exec: native tools rejected; per-turn CLI startup and harness overhead included");
    }
    // Seconds between typing and Enter. The warm start overlaps this, and a
    // cold start longer than it still shows up in the measured time.
    let think_seconds: u64 = std::env::var("BRIDGE_CHAT_SEARCH_THINK_SECONDS").ok().and_then(|value| value.parse().ok()).unwrap_or(3);
    let now = now(&corpus);
    let mut found = [0usize; 3];
    let mut totals = [0usize; 3];
    let mut deep_tokens = Vec::new();
    let mut deep_ms = Vec::new();
    let mut deep_calls = Vec::new();
    let mut fallbacks = 0;
    for query in &corpus.queries {
        let class = ["exact", "vague", "time"].iter().position(|class| *class == query.class).unwrap();
        // As the sidebar does it: the typed query hits the index, an unsure
        // answer prewarms the model, and Enter follows a moment later.
        let shallow = super::search_with(
            &core.db,
            &SearchChatsParams { query: query.query.clone(), limit: None, deep: false },
            now,
            super::DeepGate::Allowed,
            || Err("shallow".into()),
        )
        .unwrap();
        if harness == "claude" && shallow.deep_available {
            super::live::prewarm(&core, &model);
            std::thread::sleep(std::time::Duration::from_secs(think_seconds));
        }
        let result = super::search_with_budget(
            &core.db,
            &SearchChatsParams { query: query.query.clone(), limit: None, deep: true },
            now,
            super::DeepGate::Allowed,
            &budget,
            || {
                if harness == "codex" {
                    super::eval_codex::CodexEvalModel::start(&core, &model)
                        .map(|model| Box::new(model) as Box<dyn super::agent::SearchModel>)
                } else {
                    super::live::take_or_start(&core, &model)
                        .map(|model| Box::new(model) as Box<dyn super::agent::SearchModel>)
                }
            },
        )
        .unwrap();
        assert!(
            !result.detail.as_deref().is_some_and(|detail| {
                let detail = detail.to_ascii_lowercase();
                ["session limit", "usage limit", "rate_limit", "rate limit", "quota", "too many requests"]
                    .iter().any(|reason| detail.contains(reason))
            }),
            "live measurement blocked by provider quota: {:?}", result.detail
        );
        let hit = result.hits.iter().take(4).any(|hit| hit.session_id == query.expect);
        totals[class] += 1;
        found[class] += usize::from(hit);
        if result.stage != ChatSearchStage::Index {
            deep_tokens.push(result.model_tokens as u128);
            deep_ms.push(result.elapsed_ms as u128);
            deep_calls.push(result.tool_calls as u128);
            if result.stage == ChatSearchStage::IndexFallback {
                fallbacks += 1;
            }
        }
        println!(
            "{} [{}] {:?} stage={:?} tokens={} calls={} ms={} top={:?} {:?}",
            if hit { "ok  " } else { "MISS" },
            query.class,
            query.query,
            result.stage,
            result.model_tokens,
            result.tool_calls,
            result.elapsed_ms,
            result.hits.first().map(|hit| hit.session_id.as_str()),
            result.detail
        );
    }
    println!("\n| class | recall@4 (funnel, {harness} live) |\n|---|---|");
    for (index, class) in ["exact", "vague", "time"].iter().enumerate() {
        println!("| {class} | {}/{} |", found[index], totals[index]);
    }
    println!("| all | {}/{} |", found.iter().sum::<usize>(), totals.iter().sum::<usize>());
    if !deep_tokens.is_empty() {
        println!(
            "model stage ran {} times ({} fell back): median {} tokens (max {}), median {} ms (max {}), median {} lookups",
            deep_tokens.len(),
            fallbacks,
            percentile(&mut deep_tokens.clone(), 0.5),
            deep_tokens.iter().max().unwrap(),
            percentile(&mut deep_ms.clone(), 0.5),
            deep_ms.iter().max().unwrap(),
            percentile(&mut deep_calls, 0.5),
        );
    }
    let db = core.db.lock().unwrap();
    let rows: i64 = db
        .query_row("SELECT count(*) FROM usage_ledger WHERE task_family='chat_search'", [], |row| row.get(0))
        .unwrap();
    let open: i64 = db
        .query_row("SELECT count(*) FROM sessions WHERE kind='chat_search' AND ended_at IS NULL", [], |row| row.get(0))
        .unwrap();
    let forest: i64 = db
        .query_row(
            "SELECT count(*) FROM session_entries e JOIN sessions s ON s.id=e.session_id WHERE s.kind='chat_search'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    println!("usage rows tagged chat_search: {rows}; unsettled search sessions: {open}; forest entries written by search: {forest}");
    assert_eq!(open, 0, "every search session must settle");
    assert_eq!(forest, 0, "search must never write a forest entry");
    assert!(rows > 0, "provider usage must reach the ledger");
    assert!(deep_tokens.iter().any(|tokens| *tokens > 0), "no model usage measured");
}

fn live_budget() -> super::agent::Budget {
    let mut budget = super::agent::Budget::default();
    if let Ok(seconds) = std::env::var("BRIDGE_CHAT_SEARCH_LIVE_WALL_SECONDS") {
        let seconds: u64 = seconds.parse().expect("live wall seconds must be an integer");
        assert!((1..=120).contains(&seconds), "live wall seconds must be 1..=120");
        budget.wall = std::time::Duration::from_secs(seconds);
    }
    budget
}

/// Where a deep search's time goes: provider start, then each model turn.
/// `cargo test -p bridge-core --lib chat_search::eval::live_timing -- --ignored --nocapture`
#[test]
#[ignore]
fn live_timing() {
    use super::agent::SearchModel;
    let scratch = tempfile::tempdir().unwrap();
    let mut core = crate::runtime::BridgeCore::for_tests(scratch.path());
    core.adapter_registry = std::sync::Arc::new(crate::adapters::AdapterRegistry::claude_only().unwrap());
    let core = std::sync::Arc::new(core);
    let waited = Instant::now();
    while !core.adapter_registry.descriptors().iter().any(|descriptor| descriptor.id == "claude" && descriptor.available) {
        assert!(waited.elapsed().as_secs() < 60, "claude never became available");
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    for round in 0..3 {
        let started = Instant::now();
        let mut model = super::live::ClaudeSearchModel::start(&core, super::settings::DEFAULT_MODEL).unwrap();
        let start_ms = started.elapsed().as_millis();
        let mut turns = Vec::new();
        for _ in 0..3 {
            let turn_started = Instant::now();
            let reply = model
                .turn("Query: reply with {\"answer\":[]}", Instant::now() + std::time::Duration::from_secs(60))
                .unwrap();
            turns.push((turn_started.elapsed().as_millis(), reply.tokens));
        }
        println!("round {round}: start {start_ms} ms, turns {turns:?}");
    }
}

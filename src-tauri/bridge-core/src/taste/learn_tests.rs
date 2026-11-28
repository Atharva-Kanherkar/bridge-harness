use std::sync::Mutex;

use super::*;
use crate::memory_consolidation::ConsolidationOutput;
use crate::taste::replay::fixture;
use bridge_protocol::messages::{ReviewFindingDecision as Decision, ReviewSeverity};

const SCOPE: &str = "project:octo/repo";
const NIT_RULE: &str = "Skip asking to add a doc comment to `helper`";

/// The model, scripted: it answers `answer` and counts the prompts it saw.
struct Scripted {
    answer: String,
    prompts: Vec<String>,
}

impl Scripted {
    fn says(rules: serde_json::Value) -> Self {
        Self { answer: format!("Here are the rules.\n```json\n{}\n```\n", serde_json::json!({ "rules": rules })), prompts: Vec::new() }
    }
}

impl ConsolidationModel for Scripted {
    fn consolidate(&mut self, prompt: &str) -> Result<ConsolidationOutput, BridgeError> {
        self.prompts.push(prompt.to_string());
        Ok(ConsolidationOutput { text: self.answer.clone(), observed_tokens: 100, spend_microusd: 42 })
    }
}

fn open() -> (tempfile::TempDir, Mutex<Connection>) {
    let scratch = tempfile::tempdir().unwrap();
    let db = crate::store::open(&scratch.path().join("bridge.db")).unwrap();
    fixture::build(&db);
    (scratch, Mutex::new(db))
}

fn learn(db: &Mutex<Connection>, model: &mut Scripted) -> LearnReport {
    run(|| db.lock().unwrap(), model, SCOPE, fixture::day(7.0)).unwrap()
}

fn taste_records(db: &Connection, status: &str) -> Vec<MemoryRecordView> {
    crate::memory_ledger::list_layer_page(db, SCOPE, Some("taste"), Some(status), None, None)
        .unwrap()
        .records
        .into_iter()
        .map(|record| MemoryRecordView { id: record.id, kind: record.kind, body: record.body, evidence: record.evidence_count, rationale: record.rationale })
        .collect()
}

#[derive(Debug, PartialEq)]
struct MemoryRecordView {
    id: String,
    kind: String,
    body: String,
    evidence: u32,
    rationale: Option<String>,
}

#[test]
fn the_fixture_has_one_group_the_nit_dropped_on_five_prs() {
    let (_scratch, db) = open();
    let db = db.lock().unwrap();
    let found = groups(&db, SCOPE).unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].polarity, Polarity::Avoid);
    assert_eq!(found[0].signals.len(), 5);
    assert!(found[0].signals.iter().all(|signal| signal.title.as_deref() == Some(fixture::NIT)));
    // The critical finding was dropped three times too, but drops never teach
    // the reviewer to stop raising a critical or security finding.
    assert!(groups(&db, "project:octo/other").unwrap().is_empty(), "one kept nit is not a pattern");
}

#[test]
fn a_drafted_rule_lands_as_a_taste_proposal_with_its_evidence() {
    let (_scratch, db) = open();
    let mut model = Scripted::says(serde_json::json!([{
        "group": 1, "kind": "review_avoid", "rule": NIT_RULE, "pathGlobs": ["src/**"], "rationale": "Dropped on every PR."
    }]));
    let report = learn(&db, &mut model);
    assert_eq!((report.offered, report.proposed.len(), report.attached), (1, 1, 0), "{report:?}");
    let prompt = &model.prompts[0];
    assert!(prompt.contains("Group 1 (dropped 5 times on #101, #102, #103, #104, #105)"), "{prompt}");
    assert!(prompt.contains(&format!("- [dropped] {} (src/helper.rs)", fixture::NIT)), "{prompt}");

    let conn = db.lock().unwrap();
    let waiting = taste_records(&conn, "proposed");
    assert_eq!(waiting.len(), 1);
    assert_eq!((waiting[0].kind.as_str(), waiting[0].body.as_str(), waiting[0].evidence), (KIND_REVIEW_AVOID, NIT_RULE, 5));
    assert_eq!(waiting[0].rationale.as_deref(), Some("Dropped on every PR."));
    let record = crate::memory_ledger::load_record(&conn, &waiting[0].id).unwrap().unwrap();
    assert_eq!((record.layer.as_str(), record.provenance.as_str()), ("taste", "model_proposal"));
    assert_eq!(record.path_globs, Some(vec!["src/**".to_string()]));
    assert!(crate::memory_ledger::list(&conn, SCOPE, Some("proposed")).unwrap().records.is_empty(), "not a memory");
    drop(conn);

    // Its evidence is claimed: nothing is offered again, and no turn is spent.
    let mut again = Scripted::says(serde_json::json!([]));
    let report = learn(&db, &mut again);
    assert_eq!(report.offered, 0);
    assert!(again.prompts.is_empty(), "no model turn for nothing new");
}

#[test]
fn drafts_that_do_not_fit_are_refused_and_nothing_is_written() {
    let (_scratch, db) = open();
    let long = "Skip asking for doc comments ".repeat(12);
    let mut model = Scripted::says(serde_json::json!([
        { "group": 1, "kind": "review_prefer", "rule": NIT_RULE },
        { "group": 2, "kind": "review_avoid", "rule": NIT_RULE },
        { "group": 1, "kind": "review_avoid", "rule": "Use tabs for indentation everywhere" },
        { "group": 1, "kind": "review_avoid", "rule": long },
    ]));
    let report = learn(&db, &mut model);
    assert!(report.proposed.is_empty());
    assert_eq!(report.refused.len(), 4, "{:?}", report.refused);
    assert!(report.refused[0].contains("does not fit"));
    assert!(report.refused[1].contains("does not exist"));
    assert!(report.refused[2].contains("evidence's words"));
    assert!(report.refused[3].contains("characters"));
    assert!(taste_records(&db.lock().unwrap(), "proposed").is_empty());
}

#[test]
fn a_repeat_of_a_rule_the_repository_has_lends_it_the_evidence() {
    let (_scratch, db) = open();
    let existing = {
        let conn = db.lock().unwrap();
        crate::memory_ledger::insert_taste_proposal(&conn, SCOPE, KIND_REVIEW_AVOID, NIT_RULE, None, None, &[], "2026-10-01T00:00:00.000Z").unwrap()
    };
    let mut model = Scripted::says(serde_json::json!([{ "group": 1, "kind": "review_avoid", "rule": "Skip asking to add doc comment to `helper`" }]));
    let report = learn(&db, &mut model);
    assert_eq!((report.proposed.len(), report.attached), (0, 1));
    assert!(model.prompts[0].contains(&format!("Existing rules:\n- {NIT_RULE}")), "the model is told what exists");
    let conn = db.lock().unwrap();
    assert_eq!(crate::memory_ledger::load_record(&conn, &existing).unwrap().unwrap().evidence_count, 5);
}

#[test]
fn at_most_five_learned_proposals_a_fortnight() {
    let (_scratch, db) = open();
    {
        let conn = db.lock().unwrap();
        for index in 0..MAX_NEW_PROPOSALS {
            crate::memory_ledger::insert_taste_proposal(&conn, SCOPE, KIND_REVIEW_AVOID, &format!("Rule number {index} about docs"), None, None, &[], &fixture::day(1.0).to_rfc3339()).unwrap();
        }
        assert_eq!(allowance(&conn, SCOPE, fixture::day(7.0)).unwrap(), 0);
        assert_eq!(allowance(&conn, SCOPE, fixture::day(16.0)).unwrap(), MAX_NEW_PROPOSALS, "a fortnight later it is back");
    }
    let mut model = Scripted::says(serde_json::json!([{ "group": 1, "kind": "review_avoid", "rule": NIT_RULE }]));
    let report = learn(&db, &mut model);
    assert!(report.proposed.is_empty() && model.prompts.is_empty(), "no turn spent when nothing may be proposed");
}

#[test]
fn findings_kept_again_and_again_teach_how_code_should_be_written() {
    let (_scratch, db) = open();
    {
        let conn = db.lock().unwrap();
        for (index, number) in (201..=203).enumerate() {
            let at = fixture::day(10.0 + index as f64);
            let (job, ids) = fixture::review(&conn, "repo", number, &format!("sha-{number}"), at, vec![fixture::finding("src/load.rs", ReviewSeverity::Major, "Return the error with `?` instead of `unwrap()`")], vec![]);
            fixture::decide(&conn, &job, at + chrono::Duration::hours(1), &[(ids[0].as_str(), Decision::Kept, None)]);
        }
    }
    let mut model = Scripted::says(serde_json::json!([
        { "group": 1, "kind": "review_avoid", "rule": NIT_RULE },
        { "group": 2, "kind": "code_prefer", "rule": "Return the error with `?` instead of calling `unwrap()`" },
    ]));
    let report = run(|| db.lock().unwrap(), &mut model, SCOPE, fixture::day(14.0)).unwrap();
    assert_eq!(report.proposed.len(), 2, "{report:?}");
    // The three keeps join the fixture's two edits of the same `?` advice.
    assert!(model.prompts[0].contains("Group 2 (kept, edited or fixed 5 times on #101, #102, #201, #202, #203)"), "{}", model.prompts[0]);
    let kinds: Vec<String> = taste_records(&db.lock().unwrap(), "proposed").into_iter().map(|record| record.kind).collect();
    assert!(kinds.contains(&KIND_CODE_PREFER.to_string()) && kinds.contains(&KIND_REVIEW_AVOID.to_string()));
}

#[test]
fn a_group_the_model_passed_over_is_offered_again_only_once_it_grows() {
    let (_scratch, db) = open();
    let mut silent = Scripted::says(serde_json::json!([]));
    assert_eq!(learn(&db, &mut silent).offered, 1);
    let mut again = Scripted::says(serde_json::json!([]));
    assert_eq!(run(|| db.lock().unwrap(), &mut again, SCOPE, fixture::day(8.0)).unwrap().offered, 0, "nothing new since");
    {
        let conn = db.lock().unwrap();
        let at = fixture::day(9.0);
        let (job, ids) = fixture::review(&conn, "repo", 106, "sha-106", at, vec![fixture::finding("src/helper.rs", ReviewSeverity::Nit, fixture::NIT)], vec![]);
        fixture::decide(&conn, &job, at + chrono::Duration::hours(1), &[(ids[0].as_str(), Decision::Dropped, None)]);
    }
    let mut grown = Scripted::says(serde_json::json!([]));
    assert_eq!(run(|| db.lock().unwrap(), &mut grown, SCOPE, fixture::day(10.0)).unwrap().offered, 1, "a sixth drop");
}

#[test]
fn an_answer_is_read_from_its_last_json_block_or_as_bare_json() {
    assert_eq!(parse("```json\n{\"rules\":[]}\n```\nthen\n```json\n{\"rules\":[{\"group\":2,\"kind\":\"k\",\"rule\":\"r\"}]}\n```").unwrap()[0].group, 2);
    assert_eq!(parse("{\"rules\":[{\"group\":1,\"kind\":\"k\",\"rule\":\"r\"}]}").unwrap().len(), 1);
    assert!(parse("I could not find a pattern.").is_err());
    let (_scratch, db) = open();
    let mut rambling = Scripted { answer: "No rules today.".into(), prompts: Vec::new() };
    let report = learn(&db, &mut rambling);
    assert!(report.proposed.is_empty());
    let status: String = db.lock().unwrap().query_row("SELECT status FROM taste_learn_runs ORDER BY at DESC LIMIT 1", [], |row| row.get(0)).unwrap();
    assert_eq!(status, "failed");
}

#[test]
fn grounded_rules_use_the_evidence_words() {
    let evidence = vec![fixture::NIT.to_string(), "Body of: Add a doc comment to `helper`".to_string()];
    assert!(grounded(NIT_RULE, &evidence));
    assert!(!grounded("Use tabs for indentation", &evidence));
    assert!(!grounded("helper", &evidence), "one shared word is not enough");
    assert!(grounded("Do not ask for a doc comment on `helper`; stop raising missing doc comments on `helper` in src/helper.rs.", &evidence),
        "instruction words and plurals do not count against a rule");
    assert!(grounded("Do not ask for a doc comment on `helper`; missing doc comments on small helper functions are not worth raising.", &evidence));
    assert!(!grounded("Never ask to add tabs or reorder imports anywhere", &evidence), "instruction words alone ground nothing");
}

#[test]
fn a_repository_is_due_once_a_night_while_dreaming_is_on() {
    let (_scratch, db) = open();
    let conn = db.lock().unwrap();
    let night = fixture::day(7.0) + chrono::Duration::hours(14);
    let on = crate::memory_consolidation::settings(&conn, "account:local").unwrap().enabled();
    assert_eq!(due_scope(&conn, &night).unwrap().is_some(), on, "follows the dreaming switch");
    crate::memory_consolidation::update_settings(&conn, "account:local", "propose", None, None, None).unwrap();
    assert_eq!(due_scope(&conn, &night).unwrap().as_deref(), Some(SCOPE));
    record(&conn, SCOPE, &signals::instant(&night.to_rfc3339()), "skipped", &LearnReport::default(), 0, None).unwrap();
    assert_eq!(due_scope(&conn, &(night + chrono::Duration::hours(1))).unwrap(), None, "not twice a night");
    crate::memory_consolidation::update_settings(&conn, "account:local", "off", None, None, None).unwrap();
    assert_eq!(due_scope(&conn, &(night + chrono::Duration::days(1))).unwrap(), None, "off with dreaming");
}

/// One hidden bounded turn on a real harness, as the nightly run spends it.
struct Live {
    registry: std::sync::Arc<crate::adapters::AdapterRegistry>,
    scratch: std::path::PathBuf,
    harness: String,
    model: Option<String>,
}

impl ConsolidationModel for Live {
    fn consolidate(&mut self, prompt: &str) -> Result<ConsolidationOutput, BridgeError> {
        let generation =
            crate::title_jobs_live::one_hidden_turn(&self.registry, &self.scratch, &self.harness, self.model.as_deref(), "", prompt, None)
                .map_err(BridgeError::Invalid)?;
        Ok(ConsolidationOutput { text: generation.text, observed_tokens: generation.tokens, spend_microusd: generation.spend_microusd })
    }
}

/// The learner on the fixture with a real memory model. Run by hand:
///
/// ```text
/// BRIDGE_TASTE_LIVE_HARNESS=claude \
///   cargo test -p bridge-core --lib taste::learn::tests::live_learn_on_the_fixture -- --ignored --nocapture
/// ```
///
/// `BRIDGE_TASTE_LIVE_MODEL` picks a model. Expect one `review_avoid` rule
/// for the dropped nit and one `code_prefer` or `review_prefer` rule for the
/// `?` advice, both grounded; never a rule to stop raising the SQL injection.
#[test]
#[ignore = "live: runs a real harness; set BRIDGE_TASTE_LIVE_HARNESS"]
fn live_learn_on_the_fixture() {
    let harness = std::env::var("BRIDGE_TASTE_LIVE_HARNESS").expect("set BRIDGE_TASTE_LIVE_HARNESS to claude, codex or opencode");
    let (scratch, db) = open();
    {
        let conn = db.lock().unwrap();
        for (index, number) in (201..=203).enumerate() {
            let at = fixture::day(10.0 + index as f64);
            let (job, ids) = fixture::review(&conn, "repo", number, &format!("sha-{number}"), at, vec![fixture::finding("src/load.rs", ReviewSeverity::Major, "Return the error with `?` instead of `unwrap()`")], vec![]);
            fixture::decide(&conn, &job, at + chrono::Duration::hours(1), &[(ids[0].as_str(), Decision::Kept, None)]);
        }
    }
    let registry = std::sync::Arc::new(crate::adapters::AdapterRegistry::built_in().unwrap());
    std::thread::sleep(std::time::Duration::from_secs(8));
    let mut model = Live { registry, scratch: scratch.path().join("learn"), harness: harness.clone(), model: std::env::var("BRIDGE_TASTE_LIVE_MODEL").ok() };
    let started = std::time::Instant::now();
    let report = run(|| db.lock().unwrap(), &mut model, SCOPE, fixture::day(14.0)).unwrap();
    let conn = db.lock().unwrap();
    let rules = taste_records(&conn, "proposed");
    eprintln!("{harness} in {:?}: {report:#?}", started.elapsed());
    for rule in &rules {
        eprintln!("  [{}] {} (evidence {})", rule.kind, rule.body, rule.evidence);
    }
    assert!(rules.iter().any(|rule| rule.kind == KIND_REVIEW_AVOID && rule.body.to_lowercase().contains("doc")), "a rule for the dropped nit");
    assert!(rules.iter().any(|rule| rule.kind != KIND_REVIEW_AVOID && rule.body.contains('?')), "a rule for the kept `?` advice");
    assert!(rules.iter().all(|rule| !rule.body.to_lowercase().contains("injection")), "never one to stop raising a security finding");
}

#[test]
fn the_taste_view_lists_rules_across_repositories_with_the_decisions_behind_them() {
    let (_scratch, db) = open();
    let mut model = Scripted::says(serde_json::json!([{ "group": 1, "kind": "review_avoid", "rule": NIT_RULE }]));
    let id = learn(&db, &mut model).proposed[0].clone();
    let conn = db.lock().unwrap();
    let waiting = crate::memory_ledger::list_taste(&conn, Some("proposed")).unwrap();
    assert_eq!(waiting.iter().map(|rule| rule.id.as_str()).collect::<Vec<_>>(), [id.as_str()]);
    assert!(crate::memory_ledger::list_taste(&conn, None).unwrap().is_empty(), "nothing active yet");
    assert!(crate::memory_ledger::list_taste(&conn, Some("paused")).is_err());

    let evidence = crate::memory_ledger::taste_evidence(&conn, &id).unwrap();
    assert_eq!(evidence.swept, 0);
    let seen: Vec<(String, Option<u64>)> = evidence.signals.iter().map(|signal| (signal.verdict.clone(), signal.pr_number)).collect();
    assert_eq!(seen, (101..=105).map(|number| ("dropped".to_string(), Some(number))).collect::<Vec<_>>());
    assert_eq!(evidence.signals[0].title.as_deref(), Some(fixture::NIT));

    // Approved through the memory review, it is an active rule.
    crate::memory_ledger::review_batch(&conn, SCOPE, &[id.clone()], &[]).unwrap();
    assert_eq!(crate::memory_ledger::list_taste(&conn, None).unwrap()[0].id, id);
    conn.execute("DELETE FROM taste_signals WHERE pr_number=101", []).unwrap();
    assert_eq!(crate::memory_ledger::taste_evidence(&conn, &id).unwrap().swept, 1, "a swept decision is counted, not shown");
}

#[test]
fn a_rule_you_write_is_active_at_once_and_written_once() {
    let (_scratch, db) = open();
    let conn = db.lock().unwrap();
    let globs = vec!["src/**".to_string(), " ".to_string()];
    let rule = crate::memory_ledger::save_taste_rule(&conn, "project:Octo/Repo", KIND_CODE_PREFER, "No `unwrap()` outside tests", Some(&globs)).unwrap();
    assert_eq!((rule.status.as_str(), rule.provenance.as_str(), rule.layer.as_str(), rule.scope_key.as_str()), ("active", "user_explicit", "taste", SCOPE));
    assert_eq!(rule.path_globs, Some(vec!["src/**".to_string()]), "blank globs dropped");
    let again = crate::memory_ledger::save_taste_rule(&conn, SCOPE, KIND_CODE_PREFER, "no `unwrap()` outside tests", None).unwrap();
    assert_eq!(again.id, rule.id, "the same rule twice is one");
    assert!(crate::memory_ledger::save_taste_rule(&conn, "account:local", KIND_REVIEW_AVOID, "Skip nits about import order", None).is_ok(), "a rule for every repository");
    assert!(crate::memory_ledger::save_taste_rule(&conn, SCOPE, "preference", "x y", None).is_err(), "not a taste kind");
    assert!(crate::memory_ledger::save_taste_rule(&conn, "workspace:w1", KIND_REVIEW_AVOID, "x y", None).is_err(), "rules belong to a repository");
    assert!(crate::memory_ledger::list(&conn, "account:local", None).unwrap().records.is_empty(), "never a memory");
}

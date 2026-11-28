//! The taste layer's fields on the memory ledger (#1197): layers, project
//! scopes, evidence links and outcome scores. Taste rules share the ledger
//! with memories, so the property that matters most is that no memory surface
//! sees one.

use super::*;
use crate::memory_packet::{self, Retrieval};
use crate::store;
use rusqlite::params;

fn ledger_db() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let db = store::open(&dir.path().join("bridge.db")).unwrap();
    (dir, db)
}

fn memory(db: &Connection, id: &str, body: &str, status: &str) {
    db.execute(
        "INSERT INTO memory_records(id, scope_key, kind, body, provenance, status,
             valid_from, valid_to, created_at, updated_at)
         VALUES(?1, 'account:local', 'preference', ?2, 'model_proposal', ?3,
             '2026-11-01T00:00:00+00:00', CASE WHEN ?3='active' THEN NULL ELSE '2026-11-01T00:00:00+00:00' END,
             '2026-11-01T00:00:00+00:00', '2026-11-01T00:00:00+00:00')",
        params![id, body, status],
    )
    .unwrap();
}

fn rule(db: &Connection, id: &str, scope: &str, body: &str, status: &str, globs: Option<&str>, score: f64) {
    db.execute(
        "INSERT INTO memory_records(id, scope_key, kind, body, provenance, status,
             valid_from, valid_to, created_at, updated_at, layer, path_globs, outcome_score)
         VALUES(?1, ?2, 'preference', ?3, 'model_proposal', ?4,
             '2026-11-02T00:00:00+00:00', CASE WHEN ?4='active' THEN NULL ELSE '2026-11-02T00:00:00+00:00' END,
             '2026-11-02T00:00:00+00:00', '2026-11-02T00:00:00+00:00', 'taste', ?5, ?6)",
        params![id, scope, body, status, globs, score],
    )
    .unwrap();
}

fn ids(records: &[MemoryRecord]) -> Vec<&str> {
    records.iter().map(|record| record.id.as_str()).collect()
}

#[test]
fn every_record_written_without_a_layer_is_a_private_memory() {
    let (_dir, db) = ledger_db();
    let saved = save(&db, "Use bun, not npm", None, None).unwrap();
    memory(&db, "old", "Lives in Pune", "active");
    for id in [saved.id.as_str(), "old"] {
        let (layer, visibility, score, globs): (String, String, f64, Option<String>) = db
            .query_row(
                "SELECT layer, visibility, outcome_score, path_globs FROM memory_records WHERE id=?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!((layer.as_str(), visibility.as_str(), score, globs), (MEMORY_LAYER, "private", 0.0, None));
    }
    let loaded = load(&db, "old").unwrap().unwrap();
    assert_eq!((loaded.layer.as_str(), loaded.path_globs.clone(), loaded.outcome_score, loaded.evidence_count), (MEMORY_LAYER, None, 0.0, 0));
}

#[test]
fn the_schema_refuses_an_unknown_layer_or_a_shared_visibility() {
    let (_dir, db) = ledger_db();
    memory(&db, "m", "Prefers short replies", "active");
    assert!(db.execute("UPDATE memory_records SET layer='team' WHERE id='m'", []).is_err());
    assert!(db.execute("UPDATE memory_records SET visibility='team' WHERE id='m'", []).is_err());
}

#[test]
fn the_migration_fills_existing_rows_and_can_run_twice() {
    // A memory_records table as it stood at schema 104, with a row in it.
    let mut db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE memory_records(id TEXT PRIMARY KEY, scope_key TEXT NOT NULL, kind TEXT NOT NULL,
             body TEXT NOT NULL, provenance TEXT NOT NULL, status TEXT NOT NULL, updated_at TEXT NOT NULL);
         INSERT INTO memory_records VALUES('a','account:local','fact','Lives in Pune','user_explicit','active','now');",
    )
    .unwrap();
    for _ in 0..2 {
        let transaction = db.transaction().unwrap();
        install_taste_fields(&transaction).unwrap();
        transaction.commit().unwrap();
    }
    let row: (String, String, f64) = db
        .query_row("SELECT layer, visibility, outcome_score FROM memory_records WHERE id='a'", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap();
    assert_eq!(row, ("memory".into(), "private".into(), 0.0));
    let evidence: i64 = db.query_row("SELECT COUNT(*) FROM memory_evidence", [], |row| row.get(0)).unwrap();
    assert_eq!(evidence, 0);
}

#[test]
fn project_scopes_name_one_repository() {
    assert_eq!(parse_scope_key("project:Atharva-Kanherkar/bridge-harness").unwrap(), "project:atharva-kanherkar/bridge-harness");
    assert_eq!(parse_scope_key(" project:a/b.rs ").unwrap(), "project:a/b.rs");
    assert_eq!(project_scope("Org_1", "My.Repo").unwrap(), "project:org_1/my.repo");
    for refused in [
        "project:",
        "project:owner",
        "project:/repo",
        "project:owner/",
        "project:owner/repo/extra",
        "project:../repo",
        "project:owner/..",
        "project:owner/re po",
        "project:owner/repo;drop",
    ] {
        assert!(parse_scope_key(refused).is_err(), "{refused} is refused");
    }
    assert_eq!(parse_scope_key("account:local").unwrap(), ACCOUNT_MEMORY_SCOPE);
    assert!(parse_scope_key("workspace:w1").is_ok(), "workspace scopes still parse");
}

#[test]
fn layers_parse_and_nothing_else_does() {
    assert_eq!(parse_layer(None).unwrap(), MEMORY_LAYER);
    assert_eq!(parse_layer(Some(" ")).unwrap(), MEMORY_LAYER);
    assert_eq!(parse_layer(Some("taste")).unwrap(), TASTE_LAYER);
    assert!(parse_layer(Some("Taste")).is_err());
    assert!(parse_layer(Some("team")).is_err());
}

/// The same text as a taste rule beside a memory, in account and project
/// scopes, active and waiting: every memory surface reads exactly what it did
/// before the rules arrived.
#[test]
fn no_memory_surface_sees_a_taste_rule() {
    let (_dir, db) = ledger_db();
    memory(&db, "m-active", "Prefer early returns over nested ifs", "active");
    memory(&db, "m-waiting", "Runs tests with cargo nextest", "proposed");
    let before_packet = memory_packet::for_session(&db, ACCOUNT_MEMORY_SCOPE, "session-1", Retrieval::PacketOnly)
        .unwrap()
        .map(|packet| packet.text);
    let before_held = crate::memory_consolidation::held_records(&db, ACCOUNT_MEMORY_SCOPE).unwrap();
    let before_waiting = crate::memory_retention::held_proposals(&db, ACCOUNT_MEMORY_SCOPE).unwrap();
    let before_badge = crate::apps::badges::memory_proposals(&db).unwrap();

    rule(&db, "t-active", ACCOUNT_MEMORY_SCOPE, "Prefer early returns over nested ifs in reviews", "active", None, 3.0);
    rule(&db, "t-waiting", ACCOUNT_MEMORY_SCOPE, "Never flag missing doc comments on private functions", "proposed", None, 0.0);
    rule(&db, "t-project", "project:a/b", "No unwrap outside tests", "active", Some(r#"["src/**"]"#), 1.5);

    assert_eq!(ids(&list(&db, ACCOUNT_MEMORY_SCOPE, None).unwrap().records), ["m-active"]);
    assert_eq!(list(&db, ACCOUNT_MEMORY_SCOPE, None).unwrap().total, 1);
    assert_eq!(ids(&list(&db, ACCOUNT_MEMORY_SCOPE, Some("proposed")).unwrap().records), ["m-waiting"]);
    assert!(list(&db, "project:a/b", None).unwrap().records.is_empty(), "a project's rules are not its memories");
    assert_eq!(ids(&list_as_of(&db, ACCOUNT_MEMORY_SCOPE, "2026-11-20T00:00:00Z".parse().unwrap()).unwrap().records), ["m-active"]);
    assert_eq!(ids(&search(&db, ACCOUNT_MEMORY_SCOPE, "early returns", None, 0).unwrap()), ["m-active"]);
    assert!(search(&db, ACCOUNT_MEMORY_SCOPE, "doc comments", None, 0).unwrap().is_empty());
    assert_eq!(ids(&reconcilable_records(&db, ACCOUNT_MEMORY_SCOPE, 50).unwrap()), ["m-active", "m-waiting"]);
    assert!(!body_already_known(&db, ACCOUNT_MEMORY_SCOPE, "Never flag missing doc comments on private functions").unwrap());
    assert_eq!(
        near_duplicate_of(&db, ACCOUNT_MEMORY_SCOPE, "Prefer early returns over nested ifs in reviews").unwrap().map(|record| record.id),
        Some("m-active".to_string()),
        "the memory, not the rule beside it"
    );
    assert!(near_duplicate_of(&db, ACCOUNT_MEMORY_SCOPE, "Never flag missing doc comments on private functions").unwrap().is_none());
    assert_eq!(
        ids(&crate::memory_dream::dream_records(&db, ACCOUNT_MEMORY_SCOPE).unwrap().into_iter().map(|dream| dream.record).collect::<Vec<_>>()),
        ["m-active", "m-waiting"]
    );

    let after_packet = memory_packet::for_session(&db, ACCOUNT_MEMORY_SCOPE, "session-1", Retrieval::PacketOnly)
        .unwrap()
        .map(|packet| packet.text);
    assert_eq!(after_packet, before_packet, "the packet is byte-identical");
    assert_eq!(crate::memory_consolidation::held_records(&db, ACCOUNT_MEMORY_SCOPE).unwrap(), before_held, "rules do not fill the memory budget");
    assert_eq!(crate::memory_retention::held_proposals(&db, ACCOUNT_MEMORY_SCOPE).unwrap(), before_waiting, "nor the proposal allowance");
    assert_eq!(crate::apps::badges::memory_proposals(&db).unwrap(), before_badge, "nor the Memory app's badge");

    // Retiring unused memories leaves rules alone: they have their own decay.
    let ttl = crate::memory_retention::settings(&db).unwrap().proposal_ttl_days;
    crate::memory_retention::update_settings(&db, ttl, Some(30), None).unwrap();
    let retired = crate::memory_retention::sweep_unused(&db, Utc::now() + chrono::Duration::days(60)).unwrap();
    assert_eq!(retired, ["m-active"], "the unused memory retires");
    assert_eq!(load(&db, "t-active").unwrap().unwrap().status, STATUS_ACTIVE, "the rule does not");
}

#[test]
fn the_taste_layer_lists_by_itself_and_its_cursor_stays_in_it() {
    let (_dir, db) = ledger_db();
    memory(&db, "m", "Lives in Pune", "active");
    rule(&db, "t1", "project:a/b", "No unwrap outside tests", "active", Some(r#"["src/**","tests/**"]"#), 1.5);
    rule(&db, "t2", "project:a/b", "Flag bounds checks that use <=", "active", None, 0.0);
    let page = list_layer_page(&db, "project:a/b", Some("taste"), None, None, Some(1)).unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.records.len(), 1);
    let first = &page.records[0];
    assert_eq!(first.layer, TASTE_LAYER);
    let cursor = page.next_cursor.clone().expect("a second page");
    assert!(list_layer_page(&db, "project:a/b", None, None, Some(&cursor), Some(1)).is_err(), "a taste cursor does not page memories");
    let second = list_layer_page(&db, "project:a/b", Some("taste"), None, Some(&cursor), Some(1)).unwrap();
    let mut both = vec![first.id.clone(), second.records[0].id.clone()];
    both.sort();
    assert_eq!(both, ["t1", "t2"]);
    let t1 = load(&db, "t1").unwrap().unwrap();
    assert_eq!(t1.path_globs, Some(vec!["src/**".to_string(), "tests/**".to_string()]));
    assert_eq!(t1.outcome_score, 1.5);
    assert!(list_layer_page(&db, ACCOUNT_MEMORY_SCOPE, Some("taste"), None, None, None).unwrap().records.is_empty());
    assert!(list_layer_page(&db, ACCOUNT_MEMORY_SCOPE, Some("rules"), None, None, None).is_err());
}

#[test]
fn an_edit_keeps_a_rule_a_rule_with_its_evidence_paths_and_trust() {
    let (_dir, db) = ledger_db();
    rule(&db, "t", "project:a/b", "No unwrap outside tests", "active", Some(r#"["src/**"]"#), 2.5);
    add_evidence(&db, "t", "signal-1", 1.0, "2026-11-03T00:00:00+00:00").unwrap();
    add_evidence(&db, "t", "signal-2", 0.5, "2026-11-04T00:00:00+00:00").unwrap();
    add_evidence(&db, "t", "signal-2", 9.0, "2026-11-05T00:00:00+00:00").unwrap();
    assert_eq!(load(&db, "t").unwrap().unwrap().evidence_count, 2, "a signal links once");

    let edited = supersede(&db, "t", "No unwrap() outside tests and examples", None).unwrap();
    assert_eq!(edited.layer, TASTE_LAYER);
    assert_eq!(edited.path_globs, Some(vec!["src/**".to_string()]));
    assert_eq!(edited.outcome_score, 2.5);
    assert_eq!(edited.evidence_count, 2);
    let weight: f64 = db
        .query_row("SELECT weight FROM memory_evidence WHERE record_id=?1 AND signal_id='signal-2'", params![edited.id], |row| row.get(0))
        .unwrap();
    assert_eq!(weight, 0.5, "links carry over as they were");
    assert_eq!(load(&db, "t").unwrap().unwrap().status, STATUS_SUPERSEDED);
    assert!(list(&db, "project:a/b", None).unwrap().records.is_empty(), "the edit is still not a memory");
}

#[test]
fn a_merge_of_rules_keeps_every_link_the_widest_paths_and_the_best_trust() {
    let (_dir, db) = ledger_db();
    rule(&db, "a", "project:a/b", "Skip doc comments on private fns", "proposed", Some(r#"["src/**"]"#), 1.0);
    rule(&db, "b", "project:a/b", "No doc comments needed on private functions", "proposed", Some(r#"["lib/**","src/**"]"#), 2.0);
    add_evidence(&db, "a", "s1", 1.0, "2026-11-03T00:00:00+00:00").unwrap();
    add_evidence(&db, "b", "s1", 1.0, "2026-11-03T00:00:00+00:00").unwrap();
    add_evidence(&db, "b", "s2", 1.0, "2026-11-03T00:00:00+00:00").unwrap();
    let sources = vec![load(&db, "a").unwrap().unwrap(), load(&db, "b").unwrap().unwrap()];
    let merged = merge_proposals(&db, &sources, "Don't ask for doc comments on private functions", "preference", "2026-11-06T00:00:00+00:00").unwrap();
    let merged = load(&db, &merged).unwrap().unwrap();
    assert_eq!(merged.layer, TASTE_LAYER);
    assert_eq!(merged.path_globs, Some(vec!["lib/**".to_string(), "src/**".to_string()]));
    assert_eq!(merged.outcome_score, 2.0);
    assert_eq!(merged.evidence_count, 2, "s1 once, s2 once");

    rule(&db, "c", "project:a/b", "Prefer matches!", "proposed", None, 0.0);
    rule(&db, "d", "project:a/b", "Prefer matches! in match arms", "proposed", Some(r#"["src/**"]"#), 0.0);
    let sources = vec![load(&db, "c").unwrap().unwrap(), load(&db, "d").unwrap().unwrap()];
    let widest = merge_proposals(&db, &sources, "Prefer matches!", "preference", "2026-11-06T00:00:00+00:00").unwrap();
    assert_eq!(load(&db, &widest).unwrap().unwrap().path_globs, None, "a rule for every path stays one");
}

#[test]
fn a_memory_and_a_rule_never_merge() {
    let (_dir, db) = ledger_db();
    memory(&db, "m", "Prefers early returns", "proposed");
    rule(&db, "t", ACCOUNT_MEMORY_SCOPE, "Prefer early returns", "proposed", None, 0.0);
    let sources = vec![load(&db, "m").unwrap().unwrap(), load(&db, "t").unwrap().unwrap()];
    let refused = merge_proposals(&db, &sources, "Prefers early returns", "preference", "2026-11-06T00:00:00+00:00");
    assert!(refused.is_err());
    assert_eq!(load(&db, "m").unwrap().unwrap().status, STATUS_PROPOSED, "nothing changed");
    assert_eq!(load(&db, "t").unwrap().unwrap().status, STATUS_PROPOSED);
}

#[test]
fn an_edited_memory_stays_a_memory() {
    let (_dir, db) = ledger_db();
    let pin = save(&db, "Uses bun", None, None).unwrap();
    let edited = supersede(&db, &pin.id, "Uses bun for scripts and tests", None).unwrap();
    assert_eq!((edited.layer.as_str(), edited.path_globs, edited.outcome_score, edited.evidence_count), (MEMORY_LAYER, None, 0.0, 0));
}

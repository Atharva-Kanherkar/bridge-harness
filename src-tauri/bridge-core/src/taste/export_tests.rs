use super::*;

const SCOPE: &str = "project:octo/repo";

fn open() -> (tempfile::TempDir, Connection) {
    let scratch = tempfile::tempdir().unwrap();
    let db = crate::store::open(&scratch.path().join("bridge.db")).unwrap();
    (scratch, db)
}

fn rule(db: &Connection, kind: &str, body: &str, globs: Option<&[&str]>) -> MemoryRecord {
    let globs: Option<Vec<String>> = globs.map(|globs| globs.iter().map(|glob| glob.to_string()).collect());
    crate::memory_ledger::save_taste_rule(db, SCOPE, kind, body, globs.as_deref()).unwrap()
}

#[test]
fn each_file_gets_the_rules_it_carries_between_the_markers() {
    let (_scratch, db) = open();
    let code = rule(&db, "code_prefer", "No `unwrap()` outside tests", Some(&["src/**"]));
    let avoid = rule(&db, "review_avoid", "Skip doc comment nits", None);
    let rules = vec![code.clone(), avoid.clone()];
    let agents = render(Target::Agents, &rules);
    assert!(agents.starts_with(START) && agents.ends_with(&format!("{END}\n")));
    assert!(agents.contains(&format!("- No `unwrap()` outside tests (paths: src/**) <!-- id:{} kind:code_prefer -->", code.id)));
    assert!(!agents.contains("doc comment"), "code rules only");
    let bugbot = render(Target::Bugbot, &rules);
    assert!(bugbot.contains("Skip doc comment nits") && !bugbot.contains("unwrap"));
    assert!(render(Target::Claude, &[]).contains("_No rules yet._"));
}

#[test]
fn text_outside_the_markers_is_kept_byte_for_byte() {
    let section = format!("{START}\nnew\n{END}\n");
    let around = format!("# Project\n\nHand-written rules.\n\n{START}\nold\n{END}\n\n## More\nkept\n");
    let merged = merge(&around, &section);
    assert_eq!(merged, format!("# Project\n\nHand-written rules.\n\n{START}\nnew\n{END}\n\n## More\nkept\n"));
    assert_eq!(merge("# Project\nNo section yet.\n\n\n", &section), format!("# Project\nNo section yet.\n\n{section}"), "appended at the end");
    assert_eq!(merge("", &section), section);
    assert_eq!(merge(&merged, &section), merged, "merging the same section again changes nothing");
}

#[test]
fn bullets_read_back_with_their_ids_kinds_and_paths() {
    let text = format!("intro\n{START}\n## Heading\n\n- Keep functions short <!-- id:r1 kind:code_prefer -->\n- Prefer `?` (paths: src/**, lib/**)\n- \n{END}\noutro\n");
    let bullets = parse(&text).unwrap();
    assert_eq!(bullets, [
        Bullet { id: Some("r1".into()), kind: Some("code_prefer".into()), body: "Keep functions short".into(), path_globs: None },
        Bullet { id: None, kind: None, body: "Prefer `?`".into(), path_globs: Some(vec!["src/**".into(), "lib/**".into()]) },
    ]);
    assert!(parse("no section here").unwrap().is_empty());
    let broken = format!("line one\nline two\n{START}\n- Half a section\n");
    assert_eq!(parse(&broken).unwrap_err(), format!("The section that starts on line 3 has no end marker ({END})."));
}

#[test]
fn export_previews_then_writes_and_import_of_its_own_output_changes_nothing() {
    let (_scratch, db) = open();
    rule(&db, "code_prefer", "No `unwrap()` outside tests", Some(&["src/**"]));
    rule(&db, "review_avoid", "Skip doc comment nits", None);
    let checkout = tempfile::tempdir().unwrap();
    std::fs::write(checkout.path().join("AGENTS.md"), "# Agents\n\nBe kind.\n").unwrap();
    let preview = export(&db, SCOPE, checkout.path(), &Target::ALL, false).unwrap();
    assert!(preview.iter().all(|change| change.changed));
    assert!(!checkout.path().join(".cursor").exists(), "a preview writes nothing");
    assert_eq!(std::fs::read_to_string(checkout.path().join("AGENTS.md")).unwrap(), "# Agents\n\nBe kind.\n");
    export(&db, SCOPE, checkout.path(), &Target::ALL, true).unwrap();
    let agents = std::fs::read_to_string(checkout.path().join("AGENTS.md")).unwrap();
    assert!(agents.starts_with("# Agents\n\nBe kind.\n\n") && agents.contains("No `unwrap()` outside tests"));
    assert!(std::fs::read_to_string(checkout.path().join(".cursor/BUGBOT.md")).unwrap().contains("Skip doc comment nits"));
    assert!(export(&db, SCOPE, checkout.path(), &Target::ALL, false).unwrap().iter().all(|change| !change.changed), "already up to date");
    assert_eq!(import(&db, SCOPE, checkout.path(), false).unwrap(), ImportPlan::default(), "round trip");
}

#[test]
fn import_edits_named_rules_adds_new_ones_and_reports_the_missing() {
    let (_scratch, db) = open();
    let kept = rule(&db, "code_prefer", "No `unwrap()` outside tests", None);
    let gone = rule(&db, "code_prefer", "Tabs for indentation", None);
    let review = rule(&db, "review_avoid", "Skip doc comment nits", None);
    let checkout = tempfile::tempdir().unwrap();
    let agents = format!(
        "# Agents\n{START}\n- No `unwrap()` outside tests or examples <!-- id:{} kind:code_prefer -->\n- Prefer early returns (paths: src/**)\n- no `unwrap()` outside tests or examples\n{END}\n",
        kept.id
    );
    std::fs::write(checkout.path().join("AGENTS.md"), &agents).unwrap();
    let plan = import(&db, SCOPE, checkout.path(), false).unwrap();
    assert_eq!(plan.updated, [(kept.id.clone(), "No `unwrap()` outside tests or examples".to_string())]);
    assert_eq!(plan.added, ["Prefer early returns".to_string(), "no `unwrap()` outside tests or examples".to_string()]);
    assert_eq!(plan.missing, [gone.id.clone()], "the review rule is not missing from AGENTS.md: {:?}", review.id);
    assert_eq!(crate::memory_ledger::list_taste(&db, None).unwrap().len(), 3, "a preview changes nothing");

    import(&db, SCOPE, checkout.path(), true).unwrap();
    let active = crate::memory_ledger::list_taste(&db, None).unwrap();
    let bodies: Vec<&str> = active.iter().map(|rule| rule.body.as_str()).collect();
    assert!(bodies.contains(&"No `unwrap()` outside tests or examples"), "{bodies:?}");
    assert!(bodies.contains(&"Prefer early returns"));
    assert!(bodies.contains(&"Tabs for indentation"), "missing is reported, never deleted");
    let added = active.iter().find(|rule| rule.body == "Prefer early returns").unwrap();
    assert_eq!((added.kind.as_str(), added.provenance.as_str(), added.path_globs.clone()), ("code_prefer", "user_explicit", Some(vec!["src/**".to_string()])));
}

#[test]
fn a_malformed_section_is_refused_and_nothing_is_written() {
    let (_scratch, db) = open();
    rule(&db, "code_prefer", "No `unwrap()` outside tests", None);
    let checkout = tempfile::tempdir().unwrap();
    let broken = format!("# Agents\n{START}\n- half\n");
    std::fs::write(checkout.path().join("AGENTS.md"), &broken).unwrap();
    assert!(export(&db, SCOPE, checkout.path(), &[Target::Agents], true).unwrap_err().to_string().contains("no end marker"));
    assert_eq!(std::fs::read_to_string(checkout.path().join("AGENTS.md")).unwrap(), broken);
    assert!(import(&db, SCOPE, checkout.path(), true).is_err());
}

#[test]
fn the_api_finds_the_repositorys_checkout_and_says_when_there_is_none() {
    use bridge_protocol::messages::{ExportTasteRulesParams, ImportTasteRulesParams};
    let scratch = tempfile::tempdir().unwrap();
    let core = std::sync::Arc::new(crate::runtime::BridgeCore::for_tests(scratch.path()));
    let checkout = scratch.path().join("repo");
    std::fs::create_dir_all(checkout.join(".git")).unwrap();
    std::fs::write(checkout.join(".git/config"), "[remote \"origin\"]\n\turl = https://github.com/octo/repo.git\n").unwrap();
    {
        let db = core.db.lock().unwrap();
        db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','P',?1,'now')", rusqlite::params![checkout.to_string_lossy()]).unwrap();
        db.execute(
            "INSERT INTO workspaces(id,project_id,title,path,status,created_at) VALUES('w','p','Repo',?1,'idle','now')",
            rusqlite::params![checkout.to_string_lossy()],
        )
        .unwrap();
        rule(&db, "code_prefer", "No `unwrap()` outside tests", None);
    }
    let preview = crate::api::export_taste_rules(
        &core,
        &ExportTasteRulesParams { scope_key: SCOPE.into(), workspace_id: None, targets: Some(vec!["AGENTS.md".into()]), write: false },
    )
    .unwrap();
    assert_eq!((preview.workspace_id.as_str(), preview.checkouts.len(), preview.files.len()), ("w", 1, 1));
    assert!(preview.files[0].changed && !checkout.join("AGENTS.md").exists());
    crate::api::export_taste_rules(&core, &ExportTasteRulesParams { scope_key: SCOPE.into(), workspace_id: Some("w".into()), targets: None, write: true }).unwrap();
    assert!(std::fs::read_to_string(checkout.join("AGENTS.md")).unwrap().contains("No `unwrap()` outside tests"));
    let back = crate::api::import_taste_rules(&core, &ImportTasteRulesParams { scope_key: SCOPE.into(), workspace_id: None, apply: false }).unwrap();
    assert!(back.added.is_empty() && back.updated.is_empty() && back.missing.is_empty());
    let none = crate::api::export_taste_rules(
        &core,
        &ExportTasteRulesParams { scope_key: "project:octo/other".into(), workspace_id: None, targets: None, write: false },
    );
    assert!(none.unwrap_err().to_string().contains("no checkout of octo/other"));
    assert!(crate::api::export_taste_rules(
        &core,
        &ExportTasteRulesParams { scope_key: SCOPE.into(), workspace_id: None, targets: Some(vec!["README.md".into()]), write: false },
    )
    .is_err());
}

use super::fixture::{Fixture, Row, CANARY, VERSIONS};
use super::*;

fn paths(home: &Path) -> Paths {
    Paths::for_home(home)
}

/// A store at `version` with two repos (storage 3 and the legacy storage 2),
/// an active named workspace, an archived one, one with an explicit folder,
/// and chats.
fn store(home: &Path, version: i64) -> Fixture {
    let fixture = Fixture::at(&paths(home).store, version);
    fixture.repo("r-app", "app", &home.join("code/app"), 3);
    fixture.repo("r-old", "old", &home.join("code/old"), 2);
    fixture.scripts("r-app", "pnpm install", "pnpm dev", "rm -rf node_modules");
    fixture.workspace(&Row { key: "w-tokyo", repo: "r-app", directory: "tokyo", name: Some("Fix the login"), branch: Some("fix-login"), state: "active", path: None });
    fixture.workspace(&Row { key: "w-oslo", repo: "r-app", directory: "oslo", name: None, branch: Some("old-work"), state: "archived", path: None });
    fixture.workspace(&Row { key: "w-lima", repo: "r-old", directory: "lima", name: None, branch: Some("lima"), state: "active", path: None });
    fixture.session("s1", "w-tokyo", "claude");
    fixture.session("s2", "w-tokyo", "codex");
    fixture
}

#[test]
fn every_real_schema_version_reads_names_archived_state_and_scripts() {
    for &version in VERSIONS {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        store(home, version);

        let read = read(&paths(home).store).unwrap_or_else(|reason| panic!("version {version}: {reason}"));
        let found = from_store(&read, &paths(home));

        assert_eq!(read.version, version);
        let by = |name: &str| found.iter().find(|candidate| candidate.name == name).unwrap_or_else(|| panic!("version {version}: no {name} in {found:?}"));
        let tokyo = by("tokyo");
        assert_eq!(tokyo.path, home.join("conductor/workspaces/app/tokyo"), "version {version}");
        let has_names = version >= 119;
        assert_eq!(tokyo.title.as_deref(), has_names.then_some("Fix the login"), "version {version}: names arrived in migration 119");
        assert!(!tokyo.archived);
        assert_eq!(tokyo.scripts.as_ref().unwrap().setup.as_deref(), Some("pnpm install"));
        assert_eq!(tokyo.scripts.as_ref().unwrap().run_mode.as_deref(), Some("nonconcurrent"));
        assert!(by("oslo").archived, "version {version}");
        assert_eq!(by("lima").path, home.join("code/old/.conductor/lima"), "version {version}: storage 2 keeps workspaces in the repo");
        assert_eq!(read.chats.values().sum::<u64>(), 2);
    }
}

#[test]
fn conductors_own_path_rule() {
    let repo = |storage_version| Repo { id: "r".into(), name: "app".into(), root: PathBuf::from("/code/app"), storage_version, scripts: Scripts::default() };
    let root = Path::new("/Users/me/conductor/workspaces");

    assert_eq!(workspace_folder(&repo(1), "tokyo", root), Some(PathBuf::from("/code/app/tokyo")));
    assert_eq!(workspace_folder(&repo(2), "tokyo", root), Some(PathBuf::from("/code/app/.conductor/tokyo")));
    assert_eq!(workspace_folder(&repo(3), "tokyo", root), Some(PathBuf::from("/Users/me/conductor/workspaces/app/tokyo")));
    assert_eq!(workspace_folder(&repo(9), "tokyo", root), Some(PathBuf::from("/Users/me/conductor/workspaces/app/tokyo")));
    for not_a_workspace in [".", "", "  ", "..", "a/b"] {
        assert_eq!(workspace_folder(&repo(3), not_a_workspace, root), None, "{not_a_workspace:?}");
    }
}

#[test]
fn an_explicit_workspace_path_wins_over_the_rule() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let fixture = Fixture::at(&paths(home).store, 181);
    fixture.repo("r", "app", &home.join("code/app"), 3);
    let custom = home.join("elsewhere/tokyo");
    fixture.workspace(&Row { key: "w", repo: "r", directory: "tokyo", name: None, branch: Some("b"), state: "active", path: Some(&custom) });

    let found = from_store(&read(&fixture.path).unwrap(), &paths(home));

    assert_eq!(found[0].path, custom);
}

#[test]
fn a_multi_repo_workspace_is_one_bridge_workspace_per_checkout() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let fixture = Fixture::at(&paths(home).store, 181);
    fixture.repo("r-web", "web", &home.join("code/web"), 3);
    fixture.repo("r-api", "api", &home.join("code/api"), 3);
    fixture.workspace(&Row { key: "w", repo: "r-web", directory: "tokyo", name: Some("Checkout flow"), branch: Some("b"), state: "active", path: None });
    fixture.checkout("w", "r-web", None, 0);
    fixture.checkout("w", "r-api", Some(&home.join("conductor/workspaces/api/tokyo")), 1);

    let found = from_store(&read(&fixture.path).unwrap(), &paths(home));

    let titles: Vec<(Option<&str>, &Path)> = found.iter().map(|candidate| (candidate.title.as_deref(), candidate.path.as_path())).collect();
    assert_eq!(titles, vec![
        (Some("Checkout flow"), home.join("conductor/workspaces/web/tokyo").as_path()),
        (Some("Checkout flow (api)"), home.join("conductor/workspaces/api/tokyo").as_path()),
    ]);
}

#[test]
fn the_gate_fails_closed_on_any_version_or_shape_bridge_does_not_know() {
    let dir = tempfile::tempdir().unwrap();
    let newer = Fixture::new(&dir.path().join("newer"), 181);
    newer.conn.execute("INSERT INTO _sqlx_migrations(version,description,installed_on,success,checksum,execution_time) VALUES(190,'future',1,1,x'00',1)", []).unwrap();
    assert!(read(&newer.path).unwrap_err().contains("newer than Bridge knows (version 190)"));

    let older = Fixture::new(&dir.path().join("older"), 80);
    older.conn.execute("DELETE FROM _sqlx_migrations WHERE version >= 75", []).unwrap();
    assert!(read(&older.path).unwrap_err().contains("older than Bridge knows"));

    let failed = Fixture::new(&dir.path().join("failed"), 181);
    failed.conn.execute("UPDATE _sqlx_migrations SET success=0 WHERE version=181", []).unwrap();
    assert_eq!(read(&failed.path).unwrap().version, 180, "a migration that failed does not count");

    let missing = Fixture::new(&dir.path().join("missing"), 181);
    missing.conn.execute_batch("ALTER TABLE workspaces RENAME COLUMN directory_name TO folder_name").unwrap();
    assert!(read(&missing.path).unwrap_err().contains("missing what Bridge reads"));

    let garbage = dir.path().join("garbage.db");
    std::fs::write(&garbage, "not a database at all, just text").unwrap();
    assert!(read(&garbage).unwrap_err().starts_with("Bridge could not"));

    let empty = dir.path().join("empty.db");
    std::fs::write(&empty, "").unwrap();
    assert!(read(&empty).unwrap_err().contains("no version"));

    assert!(read(&dir.path().join("absent.db")).is_err());
}

#[test]
fn only_allowed_tables_are_ever_queried_and_no_secret_comes_out() {
    assert!(ALLOWED_TABLES.iter().all(|table| !DENIED_TABLES.contains(table)));
    let dir = tempfile::tempdir().unwrap();
    store(dir.path(), 181);
    STATEMENTS.with(|statements| statements.borrow_mut().clear());

    let discovery = discover(&paths(dir.path()));

    let statements = STATEMENTS.with(|statements| statements.borrow().clone());
    assert!(!statements.is_empty());
    for statement in &statements {
        let lower = statement.to_lowercase();
        for denied in DENIED_TABLES {
            assert!(!lower.contains(&format!("from {denied}")) && !lower.contains(&format!("'{denied}'")), "{statement}");
        }
        assert!(lower.starts_with("select "), "read-only statements only: {statement}");
    }
    assert!(!format!("{discovery:?}").contains(CANARY));
}

#[test]
fn the_store_is_opened_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(dir.path(), 181);
    let conn = open(&fixture.path).unwrap();
    assert!(conn.execute("DELETE FROM workspaces", []).is_err());
}

#[test]
fn folders_on_disk_are_found_without_the_database() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let paths = paths(home);
    for folder in ["conductor/workspaces/app/tokyo", "conductor/workspaces/app/oslo", "conductor/workspaces/api/lima", "conductor/workspaces/app/.hidden", "conductor/repos/app/.conductor/kyoto"] {
        std::fs::create_dir_all(home.join(folder)).unwrap();
    }
    std::fs::write(home.join("conductor/workspaces/app/notes.txt"), "a file, not a workspace").unwrap();

    let discovery = discover(&paths);

    let mut names: Vec<&str> = discovery.candidates.iter().map(|candidate| candidate.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["kyoto", "lima", "oslo", "tokyo"]);
    assert!(discovery.note.is_none());
    assert!(discovery.candidates.iter().all(|candidate| candidate.title.is_none() && !candidate.archived));
}

#[test]
fn a_folder_the_database_knows_is_listed_once_with_its_name() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    store(home, 181);
    std::fs::create_dir_all(home.join("conductor/workspaces/app/tokyo")).unwrap();
    std::fs::create_dir_all(home.join("conductor/workspaces/app/kyiv")).unwrap();
    std::fs::create_dir_all(home.join("code/old/.conductor/lima")).unwrap();

    let discovery = discover(&paths(home));

    let tokyo: Vec<&Candidate> = discovery.candidates.iter().filter(|candidate| candidate.name == "tokyo").collect();
    assert_eq!(tokyo.len(), 1);
    assert_eq!(tokyo[0].title.as_deref(), Some("Fix the login"));
    assert_eq!(discovery.candidates.iter().filter(|candidate| candidate.name == "lima").count(), 1, "a legacy folder the database knows is not listed twice");
    assert!(discovery.candidates.iter().any(|candidate| candidate.name == "kyiv" && candidate.title.is_none()), "a folder the database does not know still comes over");
    assert_eq!(discovery.repos, 2);
    assert_eq!(discovery.chats.get("codex"), Some(&1));
}

#[test]
fn a_database_bridge_cannot_read_falls_back_to_folders_and_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let fixture = store(home, 181);
    fixture.conn.execute("INSERT INTO _sqlx_migrations(version,description,installed_on,success,checksum,execution_time) VALUES(200,'future',1,1,x'00',1)", []).unwrap();
    std::fs::create_dir_all(home.join("conductor/workspaces/app/tokyo")).unwrap();

    let discovery = discover(&paths(home));

    assert!(discovery.note.as_deref().unwrap().contains("newer than Bridge knows"));
    assert_eq!(discovery.candidates.len(), 1);
    assert_eq!((discovery.candidates[0].name.as_str(), discovery.candidates[0].title.as_deref()), ("tokyo", None));
}

#[test]
fn a_database_conductor_closed_is_still_read() {
    // Once Conductor quits, SQLite removes its WAL files; a read-only open has
    // to work without them.
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let path = paths(home).store;
    {
        let fixture = store(home, 181);
        fixture.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
    }
    assert!(!path.with_extension("db-shm").exists() && !path.with_extension("db-wal").exists(), "the last connection closed cleanly");
    let before: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().flatten().map(|entry| entry.file_name()).collect();

    let read = read(&path).unwrap();

    assert_eq!(read.workspaces.len(), 3);
    let after: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().flatten().map(|entry| entry.file_name()).collect();
    assert_eq!(before, after, "reading creates no file next to Conductor's database");
}

#[test]
fn a_database_conductor_has_open_is_read_with_what_it_has_not_checkpointed() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let fixture = store(home, 181);
    fixture.conn.execute_batch("PRAGMA wal_autocheckpoint=0;").unwrap();
    fixture.workspace(&Row { key: "w-new", repo: "r-app", directory: "kyiv", name: Some("Only in the WAL"), branch: Some("k"), state: "active", path: None });

    let read = read(&fixture.path).unwrap();

    assert!(read.workspaces.iter().any(|workspace| workspace.name.as_deref() == Some("Only in the WAL")));
}


/// This Mac's real Conductor database, read as the import reads it. Prints
/// the version and counts only, and checks nothing appears beside it.
#[test]
#[ignore = "reads the real Conductor database; run by hand"]
fn read_this_macs_conductor_database() {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    let paths = Paths::for_home(&home);
    let folder = paths.store.parent().unwrap().to_path_buf();
    let before: std::collections::BTreeSet<_> = std::fs::read_dir(&folder).unwrap().flatten().map(|entry| entry.file_name()).collect();
    let discovery = discover(&paths);
    let after: std::collections::BTreeSet<_> = std::fs::read_dir(&folder).unwrap().flatten().map(|entry| entry.file_name()).collect();
    let read = read(&paths.store);
    println!(
        "MEASURE version={:?} workspaces={:?} candidates={} note={:?} files_unchanged={}",
        read.as_ref().map(|store| store.version),
        read.as_ref().map(|store| store.workspaces.len()),
        discovery.candidates.len(),
        discovery.note,
        before == after
    );
    assert_eq!(before, after);
}

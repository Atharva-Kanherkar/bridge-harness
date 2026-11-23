use super::*;
use crate::runtime::BridgeCore;
use std::fs;

pub(crate) fn git(dir: &Path, args: &[&str]) {
    // Through the counted constructor, like every git Bridge starts.
    let out = crate::git::git_command(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// A repository with one commit, at `root/<name>`.
pub(crate) fn repo(root: &Path, name: &str) -> PathBuf {
    let path = root.join(name);
    fs::create_dir_all(&path).unwrap();
    git(&path, &["init", "-q", "-b", "main"]);
    fs::write(path.join("README.md"), "hello\n").unwrap();
    git(&path, &["add", "-A"]);
    git(&path, &["commit", "-q", "-m", "first"]);
    real_path(&path)
}

/// A worktree of `repo` at `path` on a new branch.
pub(crate) fn worktree(repo: &Path, path: &Path, branch: &str) -> PathBuf {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    git(repo, &["worktree", "add", "-q", "-b", branch, &path.to_string_lossy()]);
    real_path(path)
}

pub(crate) fn candidate(path: &Path) -> Candidate {
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    Candidate {
        tool: "conductor",
        path: path.to_path_buf(),
        name: name.clone(),
        title: None,
        archived: false,
        source_ref: format!("conductor:{name}"),
        scripts: None,
    }
}

#[test]
fn git_says_what_each_folder_is() {
    let dir = tempfile::tempdir().unwrap();
    let main = repo(dir.path(), "app");
    let tokyo = worktree(&main, &dir.path().join("ws/app/tokyo"), "tokyo-branch");
    let detached = dir.path().join("ws/app/oslo");
    git(&main, &["worktree", "add", "-q", "--detach", &detached.to_string_lossy()]);
    let plain = dir.path().join("plain");
    fs::create_dir_all(&plain).unwrap();
    let copied = dir.path().join("copy");
    git(dir.path(), &["clone", "-q", &main.to_string_lossy(), &copied.to_string_lossy()]);

    assert_eq!(probe(&tokyo), Probe::OnBranch { root: main.clone(), branch: "tokyo-branch".into() });
    assert_eq!(probe(&detached), Probe::Detached { root: main.clone() });
    assert_eq!(probe(&main), Probe::MainCheckout);
    assert_eq!(probe(&plain), Probe::NotGit);
    assert_eq!(probe(&dir.path().join("gone")), Probe::Missing);
    assert_eq!(probe(&copied), Probe::MainCheckout, "a clone is its own main checkout");
}

#[test]
fn a_worktree_with_spaces_and_non_ascii_in_its_path_is_found() {
    let dir = tempfile::tempdir().unwrap();
    let main = repo(dir.path(), "my app");
    let path = worktree(&main, &dir.path().join("work spaces/my app/são paulo"), "são-paulo");

    assert_eq!(probe(&path), Probe::OnBranch { root: main, branch: "são-paulo".into() });
}

#[test]
fn every_candidate_gets_one_outcome_and_nothing_is_adopted_twice() {
    let root = PathBuf::from("/r");
    let at = |path: &str| (candidate(Path::new(path)), Probe::OnBranch { root: root.clone(), branch: "b".into() });
    let mut archived = at("/w/archived");
    archived.0.archived = true;
    let candidates = vec![
        at("/w/a"),
        at("/w/a"),
        archived,
        (candidate(Path::new("/w/gone")), Probe::Missing),
        (candidate(Path::new("/w/plain")), Probe::NotGit),
        (candidate(Path::new("/w/stray")), Probe::Unregistered),
        (candidate(Path::new("/w/detached")), Probe::Detached { root: root.clone() }),
        (candidate(Path::new("/r")), Probe::MainCheckout),
        at("/w/known"),
    ];
    let in_bridge = HashSet::from([PathBuf::from("/w/known")]);

    let outcomes = plan(&candidates, &in_bridge);

    assert_eq!(outcomes, vec![
        Outcome::Adopt { root: root.clone(), branch: "b".into() },
        Outcome::LeftOut(Reason::Duplicate),
        Outcome::LeftOut(Reason::ArchivedInTool),
        Outcome::LeftOut(Reason::Missing),
        Outcome::LeftOut(Reason::NotAWorktree),
        Outcome::LeftOut(Reason::NotAWorktree),
        Outcome::LeftOut(Reason::NeedsBranch),
        Outcome::LeftOut(Reason::MainCheckout),
        Outcome::AlreadyInBridge,
    ]);
}

fn core(scratch: &Path) -> BridgeCore {
    BridgeCore::for_tests(scratch)
}

fn workspace(db: &Connection, path: &Path) -> Option<(String, String, String, String, Option<String>)> {
    db.query_row(
        "SELECT w.id, w.title, w.branch, p.path, w.city FROM workspaces w JOIN projects p ON p.id=w.project_id WHERE w.path=?1",
        params![path.to_string_lossy()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    )
    .optional()
    .unwrap()
}

#[test]
fn an_adopted_worktree_keeps_its_folder_branch_name_and_changes() {
    let dir = tempfile::tempdir().unwrap();
    let main = repo(dir.path(), "app");
    let tokyo = worktree(&main, &dir.path().join("ws/app/tokyo"), "fix-login");
    fs::write(tokyo.join("README.md"), "changed\n").unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let core = core(scratch.path());
    let mut named = candidate(&tokyo);
    named.title = Some("Fix the login".into());
    named.scripts = Some(Scripts { setup: Some("pnpm install".into()), ..Scripts::default() });

    let report = adopt(&core.db, vec![named]).unwrap();

    assert_eq!(report.adopted.len(), 1);
    assert_eq!(report.with_scripts, 1);
    let db = core.db.lock().unwrap();
    let (id, title, branch, project, city) = workspace(&db, &tokyo).unwrap();
    assert_eq!((title.as_str(), branch.as_str(), city.as_deref()), ("Fix the login", "fix-login", Some("tokyo")));
    assert_eq!(PathBuf::from(project), main);
    assert!(is_external(&db, &id).unwrap());
    assert_eq!(fs::read_to_string(tokyo.join("README.md")).unwrap(), "changed\n", "uncommitted work is untouched");
    let scripts: String = db.query_row("SELECT scripts FROM adopted_worktrees WHERE workspace_id=?1", params![id], |row| row.get(0)).unwrap();
    assert!(scripts.contains("pnpm install"));
}

#[test]
fn adopting_again_changes_nothing_and_one_project_holds_every_worktree() {
    let dir = tempfile::tempdir().unwrap();
    let main = repo(dir.path(), "app");
    let a = worktree(&main, &dir.path().join("ws/app/a"), "a");
    let b = worktree(&main, &dir.path().join("ws/app/b"), "b");
    let scratch = tempfile::tempdir().unwrap();
    let core = core(scratch.path());

    let first = adopt(&core.db, vec![candidate(&a), candidate(&b)]).unwrap();
    let second = adopt(&core.db, vec![candidate(&a), candidate(&b)]).unwrap();

    assert_eq!((first.adopted.len(), second.adopted.len(), second.already_in_bridge), (2, 0, 2));
    let db = core.db.lock().unwrap();
    let projects: i64 = db.query_row("SELECT COUNT(*) FROM projects WHERE path=?1", params![main.to_string_lossy()], |row| row.get(0)).unwrap();
    assert_eq!(projects, 1);
}

#[test]
fn a_bridge_workspace_at_the_same_folder_is_already_in_bridge_and_stays_bridges() {
    let dir = tempfile::tempdir().unwrap();
    let main = repo(dir.path(), "app");
    let a = worktree(&main, &dir.path().join("ws/app/a"), "a");
    let scratch = tempfile::tempdir().unwrap();
    let core = core(scratch.path());
    core.db.lock().unwrap().execute(
        "INSERT INTO workspaces(id,city,title,branch,path,status,created_at) VALUES('mine','Oslo','Mine','a',?1,'idle','now')",
        params![a.to_string_lossy()],
    ).unwrap();

    let report = adopt(&core.db, vec![candidate(&a)]).unwrap();

    assert_eq!((report.adopted.len(), report.already_in_bridge), (0, 1));
    assert!(!is_external(&core.db.lock().unwrap(), "mine").unwrap());
}

#[test]
fn imported_chats_from_the_folder_move_under_the_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let main = repo(dir.path(), "app");
    let a = worktree(&main, &dir.path().join("ws/app/a"), "a");
    let scratch = tempfile::tempdir().unwrap();
    let core = core(scratch.path());
    {
        let db = core.db.lock().unwrap();
        for (id, kind, cwd) in [("imp", "imported", a.to_string_lossy().into_owned()), ("other", "imported", "/elsewhere".into()), ("live", "direct", a.to_string_lossy().into_owned())] {
            db.execute(
                "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind,cwd) VALUES(?1,NULL,'claude','x','stopped','reported',?2,?3)",
                params![id, kind, cwd],
            ).unwrap();
        }
    }

    let report = adopt(&core.db, vec![candidate(&a)]).unwrap();

    assert_eq!(report.relinked_chats, 1);
    let db = core.db.lock().unwrap();
    let under: Vec<String> = db
        .prepare("SELECT id FROM sessions WHERE workspace_id=?1")
        .unwrap()
        .query_map(params![report.adopted[0].0], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(under, vec!["imp"], "only imported chats from that folder move");
}

#[test]
fn archiving_an_adopted_workspace_in_bridge_forgets_it_and_leaves_its_folder() {
    let dir = tempfile::tempdir().unwrap();
    let main = repo(dir.path(), "app");
    let a = worktree(&main, &dir.path().join("ws/app/a"), "a");
    fs::write(a.join("notes.txt"), "uncommitted\n").unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let core = core(scratch.path());
    let report = adopt(&core.db, vec![candidate(&a)]).unwrap();
    let id = report.adopted[0].0.clone();

    core.archive_workspace(&id).unwrap();

    assert_eq!(fs::read_to_string(a.join("notes.txt")).unwrap(), "uncommitted\n");
    assert_eq!(probe(&a), Probe::OnBranch { root: main, branch: "a".into() }, "still the other tool's worktree");
    let db = core.db.lock().unwrap();
    assert!(workspace(&db, &a).is_none());
    assert!(!is_external(&db, &id).unwrap());
}

#[test]
fn archiving_a_bridge_workspace_still_removes_its_worktree() {
    let dir = tempfile::tempdir().unwrap();
    let main = repo(dir.path(), "app");
    let mine = worktree(&main, &dir.path().join("bridge/app/mine"), "mine");
    let scratch = tempfile::tempdir().unwrap();
    let core = core(scratch.path());
    {
        let db = core.db.lock().unwrap();
        db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','app',?1,'now')", params![main.to_string_lossy()]).unwrap();
        db.execute(
            "INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Mine','mine',?1,'idle','now')",
            params![mine.to_string_lossy()],
        ).unwrap();
    }

    core.archive_workspace("w").unwrap();

    assert!(!mine.exists(), "Bridge's own worktree is reclaimed as before");
}

#[test]
fn a_worktree_entry_is_on_a_branch_only_when_git_says_so_and_it_is_not_detached() {
    let entry = |branch: Option<&str>, detached: bool| git::GitWorktreeEntry {
        path: PathBuf::from("/w"),
        head: None,
        branch: branch.map(str::to_owned),
        bare: false,
        detached,
        prunable: false,
    };
    assert_eq!(branch_of(&entry(Some("fix"), false)), Some("fix".into()));
    assert_eq!(branch_of(&entry(Some("fix"), true)), None, "detached wins over a stray branch line");
    assert_eq!(branch_of(&entry(None, false)), None);
    assert_eq!(branch_of(&entry(Some(""), false)), None);
}

#[test]
fn empty_scripts_are_not_kept() {
    let dir = tempfile::tempdir().unwrap();
    let main = repo(dir.path(), "app");
    let a = worktree(&main, &dir.path().join("ws/app/a"), "a");
    let scratch = tempfile::tempdir().unwrap();
    let core = core(scratch.path());
    let mut empty = candidate(&a);
    empty.scripts = Some(Scripts { run_mode: Some("concurrent".into()), ..Scripts::default() });

    let report = adopt(&core.db, vec![empty]).unwrap();

    assert_eq!(report.with_scripts, 0, "a run mode with no scripts is nothing to keep");
    let scripts: Option<String> = core.db.lock().unwrap().query_row("SELECT scripts FROM adopted_worktrees", [], |row| row.get(0)).unwrap();
    assert_eq!(scripts, None);
    assert!(Scripts::default().is_empty());
    assert!(!Scripts { archive: Some("rm -rf build".into()), ..Scripts::default() }.is_empty());
}

#[test]
fn an_adoption_changed_bridge_only_when_a_workspace_came_or_a_chat_moved() {
    let mut report = Report::default();
    assert!(!report.imported_any());
    report.already_in_bridge = 3;
    report.left_out.insert(Reason::Missing, 2);
    assert!(!report.imported_any(), "nothing new");
    report.relinked_chats = 1;
    assert!(report.imported_any());
    report.relinked_chats = 0;
    report.adopted.push(("w".into(), PathBuf::from("/w")));
    assert!(report.imported_any());
}

#[test]
fn left_out_reasons_are_counted_each() {
    let candidates = [candidate(Path::new("/w/a")), candidate(Path::new("/w/b")), candidate(Path::new("/w/c"))];
    let outcomes = [Outcome::LeftOut(Reason::Missing), Outcome::LeftOut(Reason::Missing), Outcome::LeftOut(Reason::NeedsBranch)];
    let scratch = tempfile::tempdir().unwrap();
    let core = core(scratch.path());

    let report = write(&core.db.lock().unwrap(), &candidates, &outcomes).unwrap();

    assert_eq!(report.left_out, BTreeMap::from([(Reason::Missing, 2), (Reason::NeedsBranch, 1)]));
    assert!(report.adopted.is_empty());
}

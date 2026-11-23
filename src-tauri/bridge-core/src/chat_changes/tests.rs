use super::*;

fn git(cwd: &Path, args: &[&str]) -> String {
    let output = crate::git::git_command(cwd).args(args).output().unwrap();
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn init(dir: &Path) {
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.email", "bridge-test@example.invalid"]);
    git(dir, &["config", "user.name", "Bridge Test"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
}

fn repository() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    init(dir.path());
    std::fs::write(dir.path().join("shared.txt"), "base\n").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-qm", "seed"]);
    dir
}

fn write(repo: &Path, path: &str, contents: &str) {
    let file = repo.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, contents).unwrap();
}

fn turn(repo: &Path, session: &str, edits: impl FnOnce()) {
    record(repo, session, Phase::Start, Some("turn")).unwrap();
    edits();
    record(repo, session, Phase::End, Some("turn")).unwrap();
}

fn paths(changes: &SessionChanges) -> Vec<String> {
    changes.changes.as_ref().unwrap().files.iter().map(|file| file.path.clone()).collect()
}

#[test]
fn leaves_out_everything_that_was_already_uncommitted() {
    // The kairo shape: a checkout full of other work before the chat began.
    let repo = repository();
    write(repo.path(), "junk/notes.md", "old\n");
    write(repo.path(), "shared.txt", "someone else's edit\n");

    turn(repo.path(), "chat", || write(repo.path(), "src/new.rs", "fn main() {}\n"));

    let changes = session_changes(repo.path(), "chat", true).unwrap();
    assert!(changes.available);
    assert_eq!(paths(&changes), ["src/new.rs"]);
    let file = &changes.changes.as_ref().unwrap().files[0];
    assert_eq!(file.additions, 1);
    assert!(file.patch.contains("+fn main() {}"));
    assert_eq!(changes.turns, 1);
    assert_eq!((changes.additions, changes.deletions), (1, 0));
    assert!(changes.since.is_some());
}

#[test]
fn leaves_out_edits_made_between_its_turns() {
    let repo = repository();
    turn(repo.path(), "chat", || write(repo.path(), "a.txt", "one\n"));
    write(repo.path(), "other-agent.txt", "not this chat\n");
    turn(repo.path(), "chat", || write(repo.path(), "a.txt", "one\ntwo\n"));

    let changes = session_changes(repo.path(), "chat", true).unwrap();
    assert_eq!(paths(&changes), ["a.txt"]);
    assert_eq!(changes.changes.as_ref().unwrap().files[0].additions, 2);
    assert_eq!(changes.turns, 2);
}

#[test]
fn includes_a_turn_that_is_still_running() {
    let repo = repository();
    record(repo.path(), "chat", Phase::Start, Some("turn")).unwrap();
    write(repo.path(), "shared.txt", "base\nmid-turn\n");

    assert_eq!(paths(&session_changes(repo.path(), "chat", false).unwrap()), ["shared.txt"]);
}

#[test]
fn keeps_chats_in_one_folder_apart() {
    let repo = repository();
    turn(repo.path(), "first", || write(repo.path(), "first.txt", "1\n"));
    turn(repo.path(), "second", || write(repo.path(), "second.txt", "2\n"));

    assert_eq!(paths(&session_changes(repo.path(), "first", false).unwrap()), ["first.txt"]);
    assert_eq!(paths(&session_changes(repo.path(), "second", false).unwrap()), ["second.txt"]);
}

#[test]
fn shows_the_net_change_so_a_reverted_edit_disappears() {
    let repo = repository();
    turn(repo.path(), "chat", || write(repo.path(), "shared.txt", "changed\n"));
    turn(repo.path(), "chat", || write(repo.path(), "shared.txt", "base\n"));

    assert!(paths(&session_changes(repo.path(), "chat", false).unwrap()).is_empty());
}

#[test]
fn never_touches_the_persons_index_or_branch() {
    let repo = repository();
    write(repo.path(), "staged.txt", "staged\n");
    git(repo.path(), &["add", "staged.txt"]);
    let status_before = git(repo.path(), &["status", "--porcelain"]);
    let index_before = std::fs::read(repo.path().join(".git/index")).unwrap();
    let head_before = git(repo.path(), &["rev-parse", "HEAD"]);

    turn(repo.path(), "chat", || write(repo.path(), "new.txt", "x\n"));
    session_changes(repo.path(), "chat", true).unwrap();

    assert_eq!(std::fs::read(repo.path().join(".git/index")).unwrap(), index_before);
    assert_eq!(git(repo.path(), &["rev-parse", "HEAD"]), head_before);
    assert_eq!(git(repo.path(), &["status", "--porcelain"]), format!("{status_before}\n?? new.txt").trim_start());
    assert!(git(repo.path(), &["branch", "--list"]).lines().count() == 1);
}

#[test]
fn skips_oversized_untracked_files() {
    let repo = repository();
    turn(repo.path(), "chat", || {
        let big = vec![b'a'; (MAX_SNAPSHOT_FILE_BYTES + 1) as usize];
        std::fs::write(repo.path().join("dataset.bin"), big).unwrap();
        write(repo.path(), "small.txt", "kept\n");
    });

    assert_eq!(paths(&session_changes(repo.path(), "chat", false).unwrap()), ["small.txt"]);
}

#[test]
fn leaves_out_a_nested_repository_the_turn_created() {
    let repo = repository();
    turn(repo.path(), "chat", || {
        let nested = repo.path().join("vendor/clone");
        std::fs::create_dir_all(&nested).unwrap();
        init(&nested);
        write(&nested, "inner.txt", "x\n");
        git(&nested, &["add", "."]);
        git(&nested, &["commit", "-qm", "inner"]);
        write(repo.path(), "real.txt", "y\n");
    });

    assert_eq!(paths(&session_changes(repo.path(), "chat", false).unwrap()), ["real.txt"]);
}

#[test]
fn reports_a_rename_once() {
    let repo = repository();
    write(repo.path(), "old_name.txt", "line one\nline two\nline three\n");
    git(repo.path(), &["add", "."]);
    git(repo.path(), &["commit", "-qm", "add"]);
    turn(repo.path(), "chat", || { git(repo.path(), &["mv", "old_name.txt", "new_name.txt"]); });

    let changes = session_changes(repo.path(), "chat", true).unwrap();
    let files = &changes.changes.as_ref().unwrap().files;
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "new_name.txt");
    assert_eq!(files[0].previous_path.as_deref(), Some("old_name.txt"));
}

#[test]
fn works_before_the_first_commit() {
    let dir = tempfile::tempdir().unwrap();
    init(dir.path());
    write(dir.path(), "before.txt", "existing\n");
    turn(dir.path(), "chat", || write(dir.path(), "made.txt", "new\n"));

    assert_eq!(paths(&session_changes(dir.path(), "chat", false).unwrap()), ["made.txt"]);
}

#[test]
fn counts_without_patches_when_asked_for_stats() {
    let repo = repository();
    turn(repo.path(), "chat", || write(repo.path(), "a.txt", "1\n2\n3\n"));

    let changes = session_changes(repo.path(), "chat", false).unwrap();
    let file = &changes.changes.as_ref().unwrap().files[0];
    assert_eq!(file.additions, 3);
    assert!(file.patch.is_empty());
}

#[test]
fn says_why_there_is_no_scope() {
    let repo = repository();
    assert_eq!(
        session_changes(repo.path(), "fresh", false).unwrap().unavailable_reason,
        Some(SessionChangesUnavailable::NoTurns)
    );
    let plain = tempfile::tempdir().unwrap();
    assert_eq!(
        session_changes(plain.path(), "chat", false).unwrap().unavailable_reason,
        Some(SessionChangesUnavailable::NotGit)
    );
    record(plain.path(), "chat", Phase::Start, None).unwrap();
}

#[test]
fn an_end_with_no_recorded_start_measures_from_the_previous_snapshot() {
    // Some harnesses report a turn's end without its start.
    let repo = repository();
    turn(repo.path(), "chat", || write(repo.path(), "a.txt", "1\n"));
    write(repo.path(), "b.txt", "2\n");
    record(repo.path(), "chat", Phase::End, None).unwrap();

    assert_eq!(paths(&session_changes(repo.path(), "chat", false).unwrap()), ["a.txt", "b.txt"]);
}

#[test]
fn turns_unsafe_session_ids_into_valid_ref_names() {
    assert_eq!(ref_name("abc-123_x"), "refs/bridge/chats/abc-123_x");
    assert_eq!(ref_name("a/b..c d"), "refs/bridge/chats/a-b--c-d");
    assert_eq!(ref_name(""), "refs/bridge/chats/unnamed");
}

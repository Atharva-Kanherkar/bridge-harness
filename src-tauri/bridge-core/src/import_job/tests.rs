use super::*;
use serde_json::json;
use std::fs;

fn write(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn session(session: &str, cwd: &str, text: &str) -> String {
    [
        json!({"type":"user","uuid":format!("{session}-u"),"sessionId":session,"timestamp":"2026-11-01T10:00:00Z","cwd":cwd,"message":{"role":"user","content":text}}).to_string(),
        json!({"type":"assistant","uuid":format!("{session}-a"),"sessionId":session,"timestamp":"2026-11-01T10:00:01Z","message":{"role":"assistant","content":[{"type":"text","text":"ok"}]}}).to_string(),
    ]
    .join("\n")
}

/// A Claude home with two chats in two projects, a background run, a memory
/// and an agent.
fn claude_home(root: &Path) -> Vec<PathBuf> {
    let claude = root.join(".claude");
    write(&claude.join("projects/-work-app/s1.jsonl"), &session("s1", "/work/app", "fix the login"));
    write(&claude.join("projects/-work-api/s2.jsonl"), &session("s2", "/work/api", "add a route"));
    write(&claude.join("projects/-private-var-folders-xy-T/probe.jsonl"), &session("probe", "/private/var/folders/xy/T", "probe"));
    write(&claude.join("projects/-work-app/memory/MEMORY.md"), "- uses pnpm\n");
    write(&claude.join("agents/reviewer.md"), "---\nname: reviewer\n---\nReview diffs.\n");
    claude_import::config_roots(root, None, &[])
}

fn core(scratch: &Path) -> Arc<BridgeCore> {
    Arc::new(BridgeCore::for_tests(scratch))
}

fn new_job() -> wire::ImportJob {
    wire::ImportJob {
        job_id: format!("import_job_{}", uuid::Uuid::new_v4().simple()),
        tool: claude_import::PROVIDER.into(),
        status: "running".into(),
        started_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
        ..wire::ImportJob::default()
    }
}

/// Run a job to the end on this thread.
fn run_job(core: &Arc<BridgeCore>, roots: &[PathBuf]) -> wire::ImportJob {
    let job = new_job();
    save(core, &job, None).unwrap();
    core.import_job_running.store(true, Ordering::SeqCst);
    run(core, job, roots, None);
    status(core).unwrap().unwrap()
}

#[test]
fn what_did_not_come_over_adds_up_and_never_says_zero() {
    let mut job = new_job();
    not_carried(&mut job, "pins", 2);
    not_carried(&mut job, "tags", 0);
    not_carried(&mut job, "pins", 3);
    let items: Vec<(&str, u64)> = job.not_carried.iter().map(|item| (item.text.as_str(), item.count)).collect();
    assert_eq!(items, vec![("pins", 5)]);
}

/// Whether a `StateChanged` was published since `events` subscribed.
fn changed(events: &mut crate::events::EventReceiver) -> bool {
    let mut seen = false;
    while let Ok(event) = events.try_next() {
        seen |= matches!(event, CoreEvent::StateChanged);
    }
    seen
}

fn imported_sessions(core: &BridgeCore) -> i64 {
    let db = core.db.lock().unwrap();
    db.query_row("SELECT COUNT(*) FROM sessions WHERE kind = 'imported'", [], |row| row.get(0))
        .unwrap()
}

fn artifact(path: &str, bytes: u64) -> DiscoveredArtifact {
    DiscoveredArtifact {
        artifact_id: path.into(),
        canonical_source_ref: path.into(),
        source_label: path.into(),
        kind: CandidateKind::Conversation,
        classification: crate::external_import::SourceClassification::VersionGatedPrivate,
        stability: crate::external_import::Stability::VersionGated,
        estimated_bytes: bytes,
        modified_at: None,
        required_schema_gate: None,
        title: None,
        project_path: None,
        started_at: None,
    }
}

#[test]
fn the_work_list_is_ordered_leaves_background_runs_out_and_opens_nothing() {
    let root = tempfile::tempdir().unwrap();
    let roots = claude_home(root.path());
    let work = claude_import::job_work(&roots).unwrap();
    assert_eq!(work.left_out, 1, "the temp-folder run is left out");
    let kinds: Vec<CandidateKind> = work.artifacts.iter().map(|a| a.kind.clone()).collect();
    assert_eq!(
        kinds,
        vec![CandidateKind::Agent, CandidateKind::Conversation, CandidateKind::Conversation],
        "setup first, then chats; Claude Code keeps its own memory"
    );
    let keys: Vec<String> = work.artifacts.iter().map(claude_import::job_key).collect();
    assert!(keys.windows(2).all(|pair| pair[0] < pair[1]), "a strict order the cursor can resume from: {keys:?}");
    assert!(work.artifacts.iter().all(|a| a.title.is_none()), "no transcript is opened to build the list");
}

#[test]
fn batches_hold_at_most_25_files_and_16_mib() {
    let small: Vec<_> = (0..60).map(|i| artifact(&format!("/s/{i:02}"), 1_000)).collect();
    let sizes: Vec<usize> = batches(small).iter().map(Vec::len).collect();
    assert_eq!(sizes, vec![25, 25, 10]);

    let mib = 1024 * 1024;
    let large = vec![artifact("/a", 10 * mib), artifact("/b", 10 * mib), artifact("/c", 6 * mib), artifact("/d", 200 * mib), artifact("/e", 1)];
    let grouped: Vec<Vec<String>> = batches(large)
        .into_iter()
        .map(|batch| batch.into_iter().map(|a| a.artifact_id).collect())
        .collect();
    assert_eq!(grouped, vec![vec!["/a"], vec!["/b", "/c"], vec!["/d"], vec!["/e"]], "a file over the limit is a batch of its own");
}

#[test]
fn a_job_imports_chats_and_setup_and_leaves_background_runs_out() {
    let root = tempfile::tempdir().unwrap();
    let roots = claude_home(root.path());
    let scratch = tempfile::tempdir().unwrap();
    let core = core(scratch.path());
    let mut events = core.events.subscribe();

    let job = run_job(&core, &roots);

    assert_eq!(job.status, "done");
    assert_eq!((job.processed, job.total, job.left_out), (3, 3, 1));
    assert_eq!(job.imported, 3, "two chats and the agent: {job:?}");
    assert_eq!((job.failed, job.already_in_bridge), (0, 0));
    assert!(job.errors.is_empty(), "{:?}", job.errors);
    assert!(job.finished_at.is_some() && job.current_project.is_none());
    assert_eq!(imported_sessions(&core), 2, "the background run is not a chat in Bridge");
    assert!(!core.import_job_running.load(Ordering::SeqCst), "the job lets the next one start");

    let mut progress = Vec::new();
    loop {
        match events.try_next() {
            Ok(CoreEvent::ImportProgress(job)) => progress.push(job),
            Ok(_) => {}
            Err(_) => break,
        }
    }
    assert!(progress.len() >= 2, "progress while running, then the end");
    assert_eq!(progress.first().unwrap().status, "running");
    assert_eq!(progress.last().unwrap().status, "done");
}

#[test]
fn importing_again_brings_over_nothing_already_in_bridge() {
    let root = tempfile::tempdir().unwrap();
    let roots = claude_home(root.path());
    let scratch = tempfile::tempdir().unwrap();
    let core = core(scratch.path());
    run_job(&core, &roots);

    let again = run_job(&core, &roots);

    assert_eq!(again.status, "done");
    assert_eq!(again.imported, 0, "{again:?}");
    assert_eq!(again.already_in_bridge, 3);
    assert_eq!(imported_sessions(&core), 2);
}

#[test]
fn a_job_bridge_quit_in_the_middle_of_resumes_from_its_cursor() {
    let root = tempfile::tempdir().unwrap();
    let roots = claude_home(root.path());
    let scratch = tempfile::tempdir().unwrap();
    let core = core(scratch.path());
    // Bridge quit after the first batch: the agent was read, the rest was not.
    let work = claude_import::job_work(&roots).unwrap();
    let cursor = claude_import::job_key(&work.artifacts[0]);
    let mut job = new_job();
    job.processed = 1;
    job.total = 3;
    save(&core, &job, Some(&cursor)).unwrap();

    assert!(resume_with_roots(&core, roots.clone()).unwrap(), "a running job is picked up");
    let deadline = Instant::now() + Duration::from_secs(30);
    let finished = loop {
        let current = status(&core).unwrap().unwrap();
        if current.status != "running" {
            break current;
        }
        assert!(Instant::now() < deadline, "the resumed job never finished");
        std::thread::sleep(Duration::from_millis(20));
    };

    assert_eq!(finished.status, "done");
    assert_eq!((finished.processed, finished.total), (3, 3));
    assert_eq!(finished.imported, 2, "only what came after the cursor: {finished:?}");
    assert_eq!(imported_sessions(&core), 2);
    assert!(!resume_with_roots(&core, roots).unwrap(), "a finished job is not resumed again");
}

#[test]
fn only_one_import_runs_at_a_time() {
    let root = tempfile::tempdir().unwrap();
    let roots = claude_home(root.path());
    let scratch = tempfile::tempdir().unwrap();
    let core = core(scratch.path());
    let running = new_job();
    save(&core, &running, None).unwrap();
    core.import_job_running.store(true, Ordering::SeqCst);

    let returned = start_with_roots(&core, claude_import::PROVIDER, roots).unwrap();

    assert_eq!(returned.job_id, running.job_id, "the running job, not a second one");
}

#[test]
fn an_unreadable_chat_is_reported_and_the_rest_import() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let roots = claude_home(root.path());
    let locked = root.path().join(".claude/projects/-work-app/locked.jsonl");
    write(&locked, &session("locked", "/work/app", "secret"));
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read(&locked).is_ok() {
        return; // Running as root: nothing is unreadable.
    }
    let scratch = tempfile::tempdir().unwrap();
    let core = core(scratch.path());

    let job = run_job(&core, &roots);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o600)).unwrap();

    assert_eq!(job.status, "done");
    assert_eq!(job.failed, 1, "{job:?}");
    assert!(job.not_carried.iter().any(|item| item.text.starts_with("A chat could not be read")), "{:?}", job.not_carried);
    assert_eq!(imported_sessions(&core), 2, "the readable chats still came over");
}

/// A chat Claude Code deletes or rewrites while the import runs costs that
/// chat only: the rest of its batch still comes over.
#[test]
fn a_file_that_vanishes_mid_import_costs_only_that_file() {
    let root = tempfile::tempdir().unwrap();
    let roots = claude_home(root.path());
    let scratch = tempfile::tempdir().unwrap();
    let core = core(scratch.path());
    let work = claude_import::job_work(&roots).unwrap();
    fs::remove_file(root.path().join(".claude/projects/-work-api/s2.jsonl")).unwrap();
    let mut job = new_job();
    save(&core, &job, None).unwrap();

    Runner { core: &core, source: Source::Claude, job: &mut job, last_publish: None }.run_work(work, None).unwrap();

    assert_eq!(job.processed, 3);
    assert_eq!(job.failed, 1, "only the vanished chat: {job:?}");
    assert_eq!(job.imported, 2, "the agent and the other chat: {job:?}");
    assert_eq!(imported_sessions(&core), 1);
    assert!(job.not_carried.iter().any(|item| item.text.starts_with("A chat could not be read")), "{:?}", job.not_carried);
}

/// Claude Code in Bridge reads its own memory files, so the import leaves
/// them where they are: nothing lands in Bridge memory.
#[test]
fn claude_code_memories_stay_with_claude_code() {
    let root = tempfile::tempdir().unwrap();
    let roots = claude_home(root.path());
    let scratch = tempfile::tempdir().unwrap();
    let core = core(scratch.path());

    let job = run_job(&core, &roots);

    assert_eq!(job.status, "done");
    let records: i64 = {
        let db = core.db.lock().unwrap();
        db.query_row("SELECT COUNT(*) FROM memory_records", [], |row| row.get(0)).unwrap()
    };
    assert_eq!(records, 0);
    assert!(job.not_carried.is_empty() && job.failed == 0, "{job:?}");
}

/// An item Bridge refuses costs only itself: each family commits in its own
/// transaction, so the chats in the same batch still come over, and the
/// summary says why the item did not. (A memory over 4,000 characters is a
/// refusal that is easy to build; the job itself no longer reads memories.)
#[test]
fn one_refused_item_costs_only_itself() {
    let root = tempfile::tempdir().unwrap();
    let roots = claude_home(root.path());
    write(&root.path().join(".claude/projects/-work-app/memory/MEMORY.md"), &"- a long note\n".repeat(400));
    let mut work = claude_import::job_work(&roots).unwrap();
    let request = crate::external_import::DiscoveryRequest {
        provider: claude_import::PROVIDER.into(),
        approved_roots: roots.iter().map(|r| r.to_string_lossy().into_owned()).collect(),
        selected_export: None,
        source_version: None,
        schema_version: None,
        format_versions: std::collections::BTreeMap::new(),
    };
    let memory = ClaudeCodeImporter
        .discover_with_homes(&request, &roots)
        .unwrap()
        .artifacts
        .into_iter()
        .find(|artifact| artifact.kind == CandidateKind::Memory)
        .unwrap();
    work.artifacts.insert(1, memory);
    let scratch = tempfile::tempdir().unwrap();
    let core = core(scratch.path());
    let mut job = new_job();
    save(&core, &job, None).unwrap();

    Runner { core: &core, source: Source::Claude, job: &mut job, last_publish: None }.run_work(work, None).unwrap();

    assert_eq!((job.processed, job.failed), (4, 1), "{job:?}");
    assert_eq!(imported_sessions(&core), 2, "the chats in the same batch still came over");
    assert!(job.errors.is_empty(), "a refused item is a loss, not a job error: {:?}", job.errors);
    assert!(
        job.not_carried.iter().any(|item| item.text.starts_with("memories Bridge refused") && item.count == 1),
        "{:?}", job.not_carried
    );
}

#[test]
fn skipped_records_read_as_what_was_lost() {
    assert_eq!(skipped_record("Skipped 4 'content:document' records"), Some(("documents or PDFs inside messages".into(), 4)));
    assert_eq!(skipped_record("Skipped 2 'duplicate_source_id' records"), None, "a dropped repeat is not a loss");
    assert_eq!(
        skipped_record("Skipped 1 'record:queue-operation' records"),
        Some(("records of a kind Bridge does not know (queue-operation)".into(), 1))
    );
    assert_eq!(leading_count("3 images over 5 MB"), (3, "images over 5 MB"));
    assert_eq!(leading_count("an image over 5 MB"), (1, "an image over 5 MB"));
}

/// The whole job against this Mac's real Claude Code store, into a scratch
/// database. Prints aggregate numbers only.
#[test]
#[ignore = "reads the real ~/.claude; run by hand to measure"]
fn measure_the_job_on_this_machine() {
    let roots = claude_import::machine_config_roots();
    let started = Instant::now();
    let work = claude_import::job_work(&roots).unwrap();
    println!("MEASURE work_list_secs={:.1} files={} left_out={}", started.elapsed().as_secs_f64(), work.artifacts.len(), work.left_out);
    let scratch = tempfile::tempdir().unwrap();
    let core = core(scratch.path());
    let started = Instant::now();
    let job = run_job(&core, &roots);
    println!(
        "MEASURE job_secs={:.1} status={} processed={} imported={} already={} failed={} left_out={} not_carried_kinds={} errors={}",
        started.elapsed().as_secs_f64(), job.status, job.processed, job.imported,
        job.already_in_bridge, job.failed, job.left_out, job.not_carried.len(), job.errors.len()
    );
    for error in &job.errors {
        println!("MEASURE error: {}", error.chars().take(300).collect::<String>());
    }
    for item in &job.not_carried {
        println!("MEASURE not_carried: {} x {}", item.count, item.text.chars().take(120).collect::<String>());
    }
}

/// One real transcript (`MEASURE_FILE`), hard-linked into a scratch Claude
/// home and imported alone. Prints aggregate numbers only.
#[test]
#[ignore = "reads a real transcript; run by hand to measure"]
fn measure_one_transcript() {
    let source = PathBuf::from(std::env::var("MEASURE_FILE").expect("set MEASURE_FILE"));
    let root = tempfile::tempdir_in(source.parent().unwrap()).unwrap();
    let target = root.path().join(".claude/projects/-measure/one.jsonl");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::hard_link(&source, &target).unwrap();
    let roots = claude_import::config_roots(root.path(), None, &[]);
    let scratch = tempfile::tempdir().unwrap();
    let core = core(scratch.path());
    let started = Instant::now();
    let job = run_job(&core, &roots);
    println!("MEASURE one_secs={:.1} bytes={} status={} imported={} failed={}", started.elapsed().as_secs_f64(), fs::metadata(&source).unwrap().len(), job.status, job.imported, job.failed);
}

mod placement {
    use super::*;
    use crate::import_worktrees::tests::repo;

    fn probe(session: &str, cwd: &str) -> String {
        json!({"type":"user","uuid":format!("{session}-u"),"sessionId":session,"timestamp":"2026-11-01T10:00:00Z","cwd":cwd,"message":{"role":"user","content":"/usage"}}).to_string()
    }

    fn project_of(core: &BridgeCore, title_like: &str) -> Option<String> {
        core.db
            .lock()
            .unwrap()
            .query_row(
                "SELECT w.path FROM sessions s LEFT JOIN workspaces w ON w.id=s.workspace_id WHERE s.kind='imported' AND s.title LIKE ?1",
                params![format!("%{title_like}%")],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn bridges_own_runs_and_probes_are_left_out_and_real_chats_land_in_their_project() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().canonicalize().unwrap();
        let data = home.join("bridge-data");
        let core = Arc::new(BridgeCore::for_tests(&data));
        let app = repo(&home.join("code"), "app");
        fs::create_dir_all(app.join("src")).unwrap();
        let claude = home.join(".claude");
        let slug = crate::import_placement::claude_slug(&data.join("chats/0a1b"));
        // A Slack inbox run Bridge made: never read.
        write(&claude.join(format!("projects/{slug}/slack.jsonl")), &session("slack", &data.join("chats/0a1b").to_string_lossy(), "Check my Slack inbox"));
        // The same, under a project folder Claude Code named differently: caught by its folder.
        write(&claude.join("projects/-renamed/slack2.jsonl"), &session("slack2", &data.join("chats/0c").to_string_lossy(), "Slack unread inbox"));
        // A /usage probe.
        write(&claude.join("projects/-Users-me--claude-rimo-usage/usage.jsonl"), &probe("usage", "/Users/me/.claude/rimo-usage"));
        // Real chats: one in the repo, one in a folder that no longer exists.
        write(&claude.join("projects/-app/real.jsonl"), &session("real", &app.join("src").to_string_lossy(), "fix the login form"));
        write(&claude.join("projects/-gone/old.jsonl"), &session("old", "/work/long-gone", "an old chat"));
        let roots = claude_import::config_roots(&home, None, &[]);

        let job = run_job(&core, &roots);

        assert_eq!(job.status, "done", "{job:?}");
        assert_eq!(job.imported, 2, "the two real chats: {job:?}");
        assert_eq!(job.left_out, 3, "Bridge's two runs and the probe: {job:?}");
        assert_eq!(project_of(&core, "fix the login"), Some(app.to_string_lossy().into_owned()), "filed under the repository Bridge made a project for");
        assert_eq!(project_of(&core, "an old chat"), None, "its folder is gone");
        let projects: i64 = core.db.lock().unwrap().query_row("SELECT COUNT(*) FROM workspaces", [], |row| row.get(0)).unwrap();
        assert_eq!(projects, 1);
    }

    #[test]
    fn explicit_tidy_removes_earlier_imported_runs_and_files_chats() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().canonicalize().unwrap();
        let data = home.join("bridge-data");
        let core = Arc::new(BridgeCore::for_tests(&data));
        let app = repo(&home.join("code"), "app");
        {
            let db = core.db.lock().unwrap();
            for (id, cwd) in [("old-slack", data.join("chats/9").to_string_lossy().into_owned()), ("old-real", app.to_string_lossy().into_owned())] {
                db.execute(
                    "INSERT INTO sessions(id,harness,label,status,metric_source,kind,cwd) VALUES(?1,'claude','x','stopped','imported','imported',?2)",
                    params![id, cwd],
                )
                .unwrap();
            }
        }
        let (removed, placement) = tidy_imported_chats(&core).unwrap();
        assert_eq!(removed, 1);
        assert_eq!(placement.placed, 1);
        assert_eq!(placement.created, vec![app.clone()]);
        let left: Vec<(String, Option<String>)> = core
            .db
            .lock()
            .unwrap()
            .prepare("SELECT id, workspace_id FROM sessions WHERE kind='imported'")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].0, "old-real");
        assert!(left[0].1.is_some());
    }

    #[test]
    fn startup_maintenance_leaves_existing_imported_chats_untouched() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().canonicalize().unwrap();
        let data = home.join("bridge-data");
        let core = Arc::new(BridgeCore::for_tests(&data));
        let app = repo(&home.join("code"), "app");
        {
            let db = core.db.lock().unwrap();
            for (id, cwd) in [
                ("old-slack", data.join("chats/9")),
                ("old-real", app),
            ] {
                db.execute(
                    "INSERT INTO sessions(id,harness,label,status,metric_source,kind,cwd) VALUES(?1,'claude','x','stopped','imported','imported',?2)",
                    params![id, cwd.to_string_lossy()],
                ).unwrap();
            }
        }

        let owners = Arc::strong_count(&core);
        start_import_maintenance(core.clone());
        // Any maintenance worker owns a core clone. Wait for it to finish
        // before checking history, so the old tidy thread cannot race the
        // assertions and make this regression test pass accidentally.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while Arc::strong_count(&core) > owners {
            assert!(std::time::Instant::now() < deadline, "startup maintenance did not settle");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let db = core.db.lock().unwrap();
        let chats: Vec<(String, Option<String>)> = db
            .prepare("SELECT id, workspace_id FROM sessions WHERE kind='imported' ORDER BY id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(chats, vec![("old-real".into(), None), ("old-slack".into(), None)]);
        assert!(!core.import_job_running.load(Ordering::SeqCst));
    }
}

mod codex {
    use super::*;

    pub(super) fn line(kind: &str, payload: serde_json::Value) -> String {
        json!({ "timestamp": "2026-11-01T10:00:00Z", "ordinal": 0, "type": kind, "payload": payload }).to_string()
    }
    pub(super) fn meta(id: &str, cwd: &str, parent: Option<&str>) -> String {
        let source = match parent {
            Some(parent) => json!({ "subagent": { "thread_spawn": { "parent_thread_id": parent } } }),
            None => json!("exec"),
        };
        line("session_meta", json!({ "id": id, "cwd": cwd, "timestamp": "2026-11-01T10:00:00Z", "originator": "codex_exec", "source": source }))
    }
    pub(super) fn item(item: serde_json::Value) -> String {
        line("event_msg", json!({ "type": "item_completed", "item": item }))
    }
    pub(super) fn said(id: &str, text: &str) -> String {
        item(json!({ "type": "UserMessage", "id": id, "content": [{ "type": "text", "text": text, "text_elements": [] }] }))
    }
    pub(super) fn replied(id: &str, text: &str) -> String {
        item(json!({ "type": "AgentMessage", "id": id, "content": [{ "type": "text", "text": text }] }))
    }

    /// A Codex home: a chat with a command, its subagent in its own file, an
    /// archived chat, a background run in the temp folder, and a copy of
    /// another tool's chat nobody typed into here.
    pub(super) fn codex_home(root: &Path) -> Vec<PathBuf> {
        let home = root.join(".codex");
        let rollout = |rel: &str, lines: Vec<String>| write(&home.join(rel), &lines.join("\n"));
        rollout("sessions/2026/11/01/rollout-2026-11-01T10-00-00-t1.jsonl", vec![
            meta("t1", "/work/app", None),
            said("u1", "fix the login"),
            item(json!({ "type": "CommandExecution", "id": "c1", "command": ["/bin/zsh", "-lc", "cargo test"], "cwd": "/work/app", "aggregated_output": "ok", "exit_code": 0, "status": "completed" })),
            replied("a1", "Fixed."),
        ]);
        rollout("sessions/2026/11/01/rollout-2026-11-01T10-05-00-t2.jsonl", vec![
            meta("t2", "/work/app", Some("t1")),
            said("u2", "check the session cookie"),
            replied("a2", "The cookie is fine."),
        ]);
        rollout("archived_sessions/rollout-2026-10-01T09-00-00-t3.jsonl", vec![meta("t3", "/work/api", None), said("u3", "add a route"), replied("a3", "Added.")]);
        rollout("sessions/2026/11/02/rollout-2026-11-02T08-00-00-t4.jsonl", vec![meta("t4", "/private/var/folders/xy/T/probe", None), said("u4", "ping"), replied("a4", "pong")]);
        rollout("sessions/2026/11/02/rollout-2026-11-02T09-00-00-t5.jsonl", vec![meta("t5", "/work/app", None), replied("a5", "copied history")]);
        let source = root.join("claude-session.jsonl");
        write(&source, "{}");
        write(&home.join("external_agent_session_imports.json"), &json!({ "records": [{ "imported_thread_id": "t5", "source_path": source.to_string_lossy(), "content_sha256": "x", "imported_at": 1 }] }).to_string());
        write(&home.join("session_index.jsonl"), &json!({ "id": "t1", "thread_name": "Fix the login", "updated_at": "2026-11-01T10:10:00Z" }).to_string());
        vec![home.canonicalize().unwrap()]
    }

    pub(super) fn run_codex(core: &Arc<BridgeCore>, roots: &[PathBuf]) -> wire::ImportJob {
        let mut job = new_job();
        job.tool = crate::codex_import::PROVIDER.into();
        save(core, &job, None).unwrap();
        core.import_job_running.store(true, Ordering::SeqCst);
        run(core, job, roots, None);
        status(core).unwrap().unwrap()
    }

    fn codex_chats(core: &BridgeCore) -> Vec<(String, Option<String>, Option<String>, Option<String>)> {
        let db = core.db.lock().unwrap();
        let mut statement = db
            .prepare("SELECT title, parent_session_id, archived_at, (SELECT native_key FROM import_native_sessions n WHERE n.session_id=s.id) FROM sessions s WHERE kind='imported' AND harness='codex' ORDER BY title")
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get::<_, Option<String>>(0)?.unwrap_or_default(), row.get(1)?, row.get(2)?, row.get(3)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    fn codex_chats_come_over_with_their_children_and_archive() {
        let root = tempfile::tempdir().unwrap();
        let roots = codex_home(root.path());
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());

        let job = run_codex(&core, &roots);

        assert_eq!(job.status, "done", "{job:?}");
        assert_eq!((job.total, job.left_out), (4, 1), "the temp-folder run is left out");
        assert_eq!(job.imported, 3, "{job:?}");
        assert!(job.not_carried.iter().any(|item| item.text.contains("Codex copies of another tool's chat") && item.count == 1), "{:?}", job.not_carried);
        let chats = codex_chats(&core);
        assert_eq!(chats.len(), 3, "{chats:?}");
        let by_key = |key: &str| chats.iter().find(|chat| chat.3.as_deref() == Some(key)).cloned().unwrap();
        let (title, parent, archived, _) = by_key("codex:t1");
        assert_eq!((title.as_str(), parent, archived), ("Fix the login", None, None));
        let parent_id: String = core.db.lock().unwrap().query_row("SELECT session_id FROM import_native_sessions WHERE native_key='codex:t1'", [], |row| row.get(0)).unwrap();
        assert_eq!(by_key("codex:t2").1.as_deref(), Some(parent_id.as_str()), "the subagent is a child of its parent");
        assert!(by_key("codex:t3").2.is_some(), "the archived thread is archived");
        let shell: i64 = core.db.lock().unwrap().query_row(
            "SELECT COUNT(*) FROM session_entries WHERE session_id=?1 AND kind='tool.completed' AND payload LIKE '%\"name\":\"shell\"%' AND payload LIKE '%cargo test%'",
            params![parent_id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(shell, 1, "the command came over as a shell row");

        let again = run_codex(&core, &roots);
        assert_eq!((again.imported, again.already_in_bridge), (0, 3), "{again:?}");
    }

    #[test]
    fn a_thread_bridge_ran_is_already_in_bridge() {
        let root = tempfile::tempdir().unwrap();
        let roots = codex_home(root.path());
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());
        core.db.lock().unwrap().execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,provider_session_id,kind) VALUES('bridge-chat',NULL,'codex','Mine','stopped','reported','t1','direct')",
            [],
        ).unwrap();

        let job = run_codex(&core, &roots);

        assert_eq!((job.imported, job.already_in_bridge), (2, 1), "{job:?}");
        let parent: Option<String> = core.db.lock().unwrap().query_row(
            "SELECT parent_session_id FROM sessions WHERE id=(SELECT session_id FROM import_native_sessions WHERE native_key='codex:t2')",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(parent.as_deref(), Some("bridge-chat"), "the subagent joins the chat Bridge ran");
    }
}

mod opencode {
    use super::*;
    use crate::opencode_import::fixture::{replied, said, Fixture};

    /// An OpenCode store: a chat with a command and its subagent, an archived
    /// chat, a background run in the temp folder, and enough filler sessions
    /// that the subagent lands in a later batch than its parent.
    pub(super) fn opencode_store(data_home: &Path) -> (Fixture, Vec<PathBuf>) {
        let fixture = Fixture::new(data_home);
        fixture.session("ses_parent", None, "/work/app", Some("Fix the login"), 1_000);
        fixture.row("ses_parent", 1, "user", &said("fix the login"));
        fixture.row("ses_parent", 2, "assistant", &json!({
            "time": {}, "model": { "id": "big-pickle", "variant": "default" },
            "content": [
                { "type": "tool", "id": "call_1", "name": "shell", "state": { "status": "completed", "input": { "command": "cargo test" }, "content": [{ "type": "text", "text": "ok" }], "metadata": { "exit": 0 } } },
                { "type": "text", "text": "Fixed." }
            ]
        }));
        for index in 0..BATCH_FILES {
            let id = format!("ses_filler_{index:02}");
            fixture.session(&id, None, "/work/app", Some(&format!("Filler {index:02}")), 2_000 + index as i64);
            fixture.row(&id, 1, "user", &said("hello"));
            fixture.row(&id, 2, "assistant", &replied("hi"));
        }
        fixture.session("ses_child", Some("ses_parent"), "/work/app", Some("Audit (@general subagent)"), 9_000);
        fixture.row("ses_child", 1, "user", &said("audit the tests"));
        fixture.row("ses_child", 2, "assistant", &replied("All good."));
        fixture.session("ses_old", None, "/work/api", Some("Add a route"), 500);
        fixture.row("ses_old", 1, "user", &said("add a route"));
        fixture.row("ses_old", 2, "assistant", &replied("Added."));
        fixture.archive("ses_old");
        fixture.session("ses_temp", None, "/private/var/folders/xy/T/probe", Some("probe"), 9_500);
        fixture.row("ses_temp", 1, "user", &said("ping"));
        let roots = vec![fixture.home.clone()];
        (fixture, roots)
    }

    pub(super) fn run_opencode(core: &Arc<BridgeCore>, roots: &[PathBuf]) -> wire::ImportJob {
        let mut job = new_job();
        job.tool = crate::opencode_import::PROVIDER.into();
        save(core, &job, None).unwrap();
        core.import_job_running.store(true, Ordering::SeqCst);
        run(core, job, roots, None);
        status(core).unwrap().unwrap()
    }

    fn session_of(core: &BridgeCore, key: &str) -> (String, Option<String>, Option<String>) {
        core.db.lock().unwrap().query_row(
            "SELECT s.id, s.parent_session_id, s.archived_at FROM sessions s JOIN import_native_sessions n ON n.session_id=s.id WHERE n.native_key=?1",
            params![key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap()
    }

    #[test]
    fn opencode_chats_come_over_with_their_children_and_archive() {
        let root = tempfile::tempdir().unwrap();
        let (_fixture, roots) = opencode_store(root.path());
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());

        let job = run_opencode(&core, &roots);

        let chats = BATCH_FILES as u64 + 3;
        assert_eq!(job.status, "done", "{job:?}");
        assert_eq!((job.total, job.left_out, job.imported), (chats, 1, chats), "{job:?}");
        let (parent, parent_of_parent, _) = session_of(&core, "opencode:ses_parent");
        assert_eq!(parent_of_parent, None);
        let (_, child_parent, _) = session_of(&core, "opencode:ses_child");
        assert_eq!(child_parent.as_deref(), Some(parent.as_str()), "the subagent, read in a later batch, is a child of its parent");
        assert!(session_of(&core, "opencode:ses_old").2.is_some(), "the archived session is archived");
        let (harness, title): (String, String) = core.db.lock().unwrap().query_row(
            "SELECT harness, title FROM sessions WHERE id=?1", params![parent], |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!((harness.as_str(), title.as_str()), ("opencode", "Fix the login"));
        let shell: i64 = core.db.lock().unwrap().query_row(
            "SELECT COUNT(*) FROM session_entries WHERE session_id=?1 AND kind='tool.completed' AND payload LIKE '%\"name\":\"bash\"%' AND payload LIKE '%cargo test%'",
            params![parent],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(shell, 1, "the command came over as a shell row");

        let again = run_opencode(&core, &roots);
        assert_eq!((again.imported, again.already_in_bridge), (0, chats), "{again:?}");
    }

    #[test]
    fn a_session_bridge_ran_is_already_in_bridge() {
        let root = tempfile::tempdir().unwrap();
        let (_fixture, roots) = opencode_store(root.path());
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());
        core.db.lock().unwrap().execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,provider_session_id,kind) VALUES('bridge-chat',NULL,'opencode','Mine','stopped','reported','ses_parent','direct')",
            [],
        ).unwrap();

        let job = run_opencode(&core, &roots);

        assert_eq!(job.already_in_bridge, 1, "{job:?}");
        assert_eq!(session_of(&core, "opencode:ses_child").1.as_deref(), Some("bridge-chat"), "the subagent joins the chat Bridge ran");
    }

    #[test]
    fn a_job_quit_in_the_middle_resumes_after_its_cursor() {
        let root = tempfile::tempdir().unwrap();
        let (_fixture, roots) = opencode_store(root.path());
        let work = crate::opencode_import::job_work(&roots).unwrap();
        let cursor = Source::OpenCode.key(&work.artifacts[2]);
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());
        let mut job = new_job();
        job.tool = crate::opencode_import::PROVIDER.into();
        job.processed = 3;
        save(&core, &job, Some(&cursor)).unwrap();

        assert!(resume_with_roots(&core, roots.clone()).unwrap());
        let started = Instant::now();
        while core.import_job_running.load(Ordering::SeqCst) && started.elapsed() < Duration::from_secs(20) {
            std::thread::sleep(Duration::from_millis(20));
        }

        let finished = status(&core).unwrap().unwrap();
        assert_eq!(finished.status, "done", "{finished:?}");
        assert_eq!(finished.imported, work.artifacts.len() as u64 - 3, "only what came after the cursor");
    }
}

/// Accounts added in Settings, each with its own folder.
mod accounts {
    use super::*;
    use crate::opencode_import::fixture::{replied, said, Fixture};

    fn add_accounts(core: &BridgeCore, harness: &str, accounts: serde_json::Value) {
        let db = core.db.lock().unwrap();
        let mut config = crate::agent_config::state(&db).unwrap().harnesses.into_iter().find(|item| item.id == harness).unwrap();
        config.advanced = json!({ "accounts": accounts });
        crate::agent_config::save_harness(&db, config).unwrap();
    }

    /// The homes a job on a machine whose home folder is `home` reads.
    fn roots(core: &BridgeCore, home: &Path, source: Source) -> Vec<PathBuf> {
        let host = HostHomes { home: home.to_path_buf(), claude_config_dir: None, codex_home: None, xdg_data_home: None };
        let homes = crate::import_continue::sign_in_homes_with(&core.db.lock().unwrap(), &host).unwrap();
        source.roots(&host, &homes)
    }

    fn run_tool(core: &Arc<BridgeCore>, tool: &str, roots: &[PathBuf]) -> wire::ImportJob {
        let mut job = new_job();
        job.tool = tool.into();
        save(core, &job, None).unwrap();
        core.import_job_running.store(true, Ordering::SeqCst);
        run(core, job, roots, None);
        status(core).unwrap().unwrap()
    }

    fn native_keys(core: &BridgeCore) -> Vec<String> {
        let db = core.db.lock().unwrap();
        let mut statement = db.prepare("SELECT native_key FROM import_native_sessions ORDER BY native_key").unwrap();
        statement.query_map([], |row| row.get(0)).unwrap().map(Result::unwrap).collect()
    }

    #[test]
    fn a_claude_account_folder_is_read_with_your_own() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().canonicalize().unwrap();
        write(&home.join(".claude/projects/-work-app/s1.jsonl"), &session("s1", "/work/app", "fix the login"));
        let work = home.join("claude-work");
        write(&work.join("projects/-work-api/s2.jsonl"), &session("s2", "/work/api", "add a route"));
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());
        add_accounts(&core, "claude", json!([
            { "id": "work", "label": "Work", "homePath": work.to_string_lossy() },
            { "id": "gone", "label": "Gone", "homePath": home.join("never-made").to_string_lossy() },
        ]));

        let roots = roots(&core, &home, Source::Claude);
        assert_eq!(roots, vec![home.join(".claude"), work.clone()], "your own folder, then the account's; a missing one is skipped");
        let job = run_tool(&core, claude_import::PROVIDER, &roots);

        assert_eq!((job.status.as_str(), job.imported), ("done", 2), "{job:?}");
        assert_eq!(native_keys(&core), vec!["claude:s1", "claude:s2"]);
        let again = run_tool(&core, claude_import::PROVIDER, &roots);
        assert_eq!((again.imported, again.already_in_bridge), (0, 2), "{again:?}");
    }

    #[test]
    fn an_account_pointing_at_your_own_folder_is_read_once() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().canonicalize().unwrap();
        write(&home.join(".claude/projects/-work-app/s1.jsonl"), &session("s1", "/work/app", "fix the login"));
        let alias = home.join("claude-alias");
        std::os::unix::fs::symlink(home.join(".claude"), &alias).unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());
        add_accounts(&core, "claude", json!([{ "id": "alias", "label": "Alias", "homePath": alias.to_string_lossy() }]));

        assert_eq!(roots(&core, &home, Source::Claude), vec![home.join(".claude")]);
    }

    #[test]
    fn a_codex_account_home_is_read_with_your_own() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().canonicalize().unwrap();
        let rollout = |codex: &Path, id: &str, text: &str| {
            let line = |kind: &str, payload: serde_json::Value| json!({ "timestamp": "2026-11-01T10:00:00Z", "type": kind, "payload": payload }).to_string();
            write(&codex.join(format!("sessions/2026/11/01/rollout-2026-11-01T10-00-00-{id}.jsonl")), &[
                line("session_meta", json!({ "id": id, "cwd": "/work/app", "timestamp": "2026-11-01T10:00:00Z", "source": "exec" })),
                line("event_msg", json!({ "type": "item_completed", "item": { "type": "UserMessage", "id": format!("{id}-u"), "content": [{ "type": "text", "text": text }] } })),
                line("event_msg", json!({ "type": "item_completed", "item": { "type": "AgentMessage", "id": format!("{id}-a"), "content": [{ "type": "text", "text": "Done." }] } })),
            ].join("\n"));
        };
        rollout(&home.join(".codex"), "t-own", "fix the login");
        let work = home.join("codex-work");
        rollout(&work, "t-work", "add a route");
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());
        add_accounts(&core, "codex", json!([
            { "id": "work", "label": "Work", "homePath": work.to_string_lossy() },
            { "id": "shadow", "label": "Shadow", "shadowHomePath": home.join("codex-shadow").to_string_lossy() },
        ]));

        let roots = roots(&core, &home, Source::Codex);
        assert_eq!(roots, vec![home.join(".codex"), work], "a login overlay shares your own home");
        let job = run_tool(&core, codex_import::PROVIDER, &roots);

        assert_eq!((job.status.as_str(), job.imported), ("done", 2), "{job:?}");
        assert_eq!(native_keys(&core), vec!["codex:t-own", "codex:t-work"]);
    }

    #[test]
    fn an_opencode_account_store_is_read_with_your_own() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().canonicalize().unwrap();
        let own = Fixture::new(&home.join(".local/share"));
        own.session("ses_own", None, "/work/app", Some("Fix the login"), 1);
        own.row("ses_own", 1, "user", &said("fix the login"));
        own.row("ses_own", 2, "assistant", &replied("Done."));
        let work_data = home.join("opencode-work");
        let work = Fixture::new(&work_data);
        work.session("ses_work", None, "/work/api", Some("Add a route"), 2);
        work.row("ses_work", 1, "user", &said("add a route"));
        work.row("ses_work", 2, "assistant", &replied("Added."));
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());
        add_accounts(&core, "opencode", json!([{ "id": "work", "label": "Work", "homePath": work_data.to_string_lossy() }]));

        let roots = roots(&core, &home, Source::OpenCode);
        assert_eq!(roots, vec![own.home.clone(), work.home.clone()]);
        let job = run_tool(&core, opencode_import::PROVIDER, &roots);

        assert_eq!((job.status.as_str(), job.imported), ("done", 2), "{job:?}");
        assert_eq!(native_keys(&core), vec!["opencode:ses_own", "opencode:ses_work"]);
    }

    #[test]
    fn with_no_accounts_only_your_own_folders_are_read() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().canonicalize().unwrap();
        write(&home.join(".claude/projects/-work-app/s1.jsonl"), &session("s1", "/work/app", "fix the login"));
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());

        assert_eq!(roots(&core, &home, Source::Claude), vec![home.join(".claude")]);
        assert!(roots(&core, &home, Source::Codex).is_empty());
        assert!(roots(&core, &home, Source::OpenCode).is_empty());
    }
}

/// OpenCode's job against this Mac's real store, into a scratch database.
/// Prints aggregate numbers only.
#[test]
#[ignore = "reads the real OpenCode store; run by hand to measure"]
fn measure_the_opencode_job_on_this_machine() {
    let roots = crate::opencode_import::machine_opencode_homes();
    let started = Instant::now();
    let work = crate::opencode_import::job_work(&roots).unwrap();
    println!("MEASURE work_list_secs={:.2} sessions={} left_out={}", started.elapsed().as_secs_f64(), work.artifacts.len(), work.left_out);
    let scratch = tempfile::tempdir().unwrap();
    let core = core(scratch.path());
    let started = Instant::now();
    let job = opencode::run_opencode(&core, &roots);
    println!(
        "MEASURE job_secs={:.1} status={} processed={} imported={} already={} failed={} left_out={} errors={}",
        started.elapsed().as_secs_f64(), job.status, job.processed, job.imported, job.already_in_bridge, job.failed, job.left_out, job.errors.len()
    );
    for item in &job.not_carried {
        println!("MEASURE not_carried: {} x {}", item.count, item.text.chars().take(120).collect::<String>());
    }
    for error in &job.errors {
        println!("MEASURE error: {}", error.chars().take(200).collect::<String>());
    }
    let db = core.db.lock().unwrap();
    let count = |sql: &str| db.query_row(sql, [], |row| row.get::<_, i64>(0)).unwrap();
    println!(
        "MEASURE tool_rows={} child_chats={} messages={}",
        count("SELECT COUNT(*) FROM session_entries WHERE kind='tool.completed'"),
        count("SELECT COUNT(*) FROM sessions WHERE kind='imported' AND parent_session_id IS NOT NULL"),
        count("SELECT COUNT(*) FROM session_entries"),
    );
}

mod conductor {
    use super::*;
    use crate::conductor_import::fixture::{Fixture, Row};
    use crate::import_worktrees::tests::{git, repo, worktree};

    /// A home where Conductor holds, for one repo: an active workspace with a
    /// chat imported from Claude Code, one archived in Conductor, one on a
    /// detached HEAD, one whose folder is gone, and a stray folder.
    fn home(root: &Path) -> (PathBuf, PathBuf) {
        let home = root.canonicalize().unwrap();
        let main = repo(&home.join("code"), "app");
        let tokyo = worktree(&main, &home.join("conductor/workspaces/app/tokyo"), "fix-login");
        worktree(&main, &home.join("conductor/workspaces/app/oslo"), "old-work");
        let lima = home.join("conductor/workspaces/app/lima");
        git(&main, &["worktree", "add", "-q", "--detach", &lima.to_string_lossy()]);
        fs::create_dir_all(home.join("conductor/workspaces/app/stray")).unwrap();
        let fixture = Fixture::at(&crate::conductor_import::Paths::for_home(&home).store, 181);
        fixture.repo("r", "app", &main, 3);
        fixture.scripts("r", "pnpm install", "pnpm dev", "");
        for (key, directory, name, state) in [("a", "tokyo", Some("Fix the login"), "active"), ("b", "oslo", None, "archived"), ("c", "lima", None, "active"), ("d", "kyiv", None, "active")] {
            fixture.workspace(&Row { key, repo: "r", directory, name, branch: Some(directory), state, path: None });
        }
        (home, tokyo)
    }

    fn run_conductor(core: &Arc<BridgeCore>, home: &Path) -> wire::ImportJob {
        let mut job = new_job();
        job.tool = crate::conductor_import::PROVIDER.into();
        save(core, &job, None).unwrap();
        core.import_job_running.store(true, Ordering::SeqCst);
        run(core, job, &[home.to_path_buf()], None);
        status(core).unwrap().unwrap()
    }

    #[test]
    fn conductors_workspaces_come_over_in_place_and_say_what_was_left() {
        let root = tempfile::tempdir().unwrap();
        let (home, tokyo) = home(root.path());
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());
        core.db.lock().unwrap().execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind,cwd) VALUES('chat',NULL,'claude','x','stopped','reported','imported',?1)",
            params![tokyo.to_string_lossy()],
        ).unwrap();

        let job = run_conductor(&core, &home);

        assert_eq!(job.status, "done", "{job:?}");
        assert_eq!((job.total, job.imported, job.left_out), (5, 1, 1), "{job:?}");
        let texts: Vec<(&str, u64)> = job.not_carried.iter().map(|item| (item.text.as_str(), item.count)).collect();
        for expected in [
            ("workspaces on a detached HEAD, which need a branch first", 1),
            ("workspaces whose folder is gone", 1),
            ("folders that are not git worktrees", 1),
        ] {
            assert!(texts.contains(&expected), "{expected:?} in {texts:?}");
        }
        let db = core.db.lock().unwrap();
        let (id, title): (String, String) = db.query_row("SELECT id, title FROM workspaces WHERE path=?1", params![tokyo.to_string_lossy()], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
        assert_eq!(title, "Fix the login");
        let chat: Option<String> = db.query_row("SELECT workspace_id FROM sessions WHERE id='chat'", [], |row| row.get(0)).unwrap();
        assert_eq!(chat.as_deref(), Some(id.as_str()), "the imported chat moved under its workspace");
        drop(db);

        let again = run_conductor(&core, &home);
        assert_eq!((again.imported, again.already_in_bridge), (0, 1), "{again:?}");
        assert!(tokyo.join("README.md").is_file(), "Conductor's folder is untouched");
    }

    #[test]
    fn a_conductor_database_bridge_does_not_know_still_brings_the_folders() {
        let root = tempfile::tempdir().unwrap();
        let (home, tokyo) = home(root.path());
        let store = crate::conductor_import::Paths::for_home(&home).store;
        rusqlite::Connection::open(&store).unwrap().execute(
            "INSERT INTO _sqlx_migrations(version,description,installed_on,success,checksum,execution_time) VALUES(250,'future',1,1,x'00',1)",
            [],
        ).unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());

        let job = run_conductor(&core, &home);

        assert_eq!(job.status, "done", "{job:?}");
        assert_eq!(job.imported, 2, "tokyo and oslo by folder; archived is unknown without the database: {job:?}");
        assert!(job.not_carried.iter().any(|item| item.text.contains("newer than Bridge knows")), "{:?}", job.not_carried);
        let title: String = core.db.lock().unwrap().query_row("SELECT title FROM workspaces WHERE path=?1", params![tokyo.to_string_lossy()], |row| row.get(0)).unwrap();
        assert_eq!(title, "tokyo");
    }
}

mod t3code {
    use super::codex::{meta, replied, said};
    use super::*;
    use crate::import_worktrees::tests::{repo, worktree};
    use crate::t3code_import::fixture::{Fixture, ThreadRow};

    /// A Claude Code session whose transcript is still there.
    const KEPT: &str = "11111111-1111-4111-8111-111111111111";
    /// One Claude Code deleted (it keeps 30 days by default).
    const GONE: &str = "22222222-2222-4222-8222-222222222222";
    const CODEX: &str = "01a1f9f8-cbdc-7a20-a62a-3e214132a143";

    struct Home {
        t3: PathBuf,
        claude: Vec<PathBuf>,
        codex: Vec<PathBuf>,
        worktree: PathBuf,
        main: PathBuf,
    }

    /// A home where T3 Code ran, for one repo: a Claude thread in a worktree
    /// whose transcript is still there, an archived and pinned Claude
    /// thread whose transcript is gone, a Codex thread, and a deleted one.
    fn home(root: &Path) -> Home {
        let home = root.canonicalize().unwrap();
        let main = repo(&home.join("code"), "shop");
        let tree = worktree(&main, &home.join(".t3/worktrees/shop/feat-discount-07db51fef00c511d"), "feat/discount");
        write(&home.join(format!(".claude/projects/-shop/{KEPT}.jsonl")), &session(KEPT, &tree.to_string_lossy(), "add a discount helper"));
        let codex_home = home.join(".codex");
        write(
            &codex_home.join(format!("sessions/2026/11/20/rollout-2026-11-20T17-19-54-{CODEX}.jsonl")),
            // Not the test's temp folder: Codex's import leaves chats run there out as background runs.
            &[meta(CODEX, "/work/shop", None), said("u1", "round prices"), replied("a1", "Rounded.")].join("\n"),
        );
        let fixture = Fixture::in_base(&home.join(".t3"), 102);
        fixture.project("p", &main, "shop");
        let row = |id, title, driver, native| ThreadRow { id, project: "p", title, driver, native, updated_at: "2026-11-20T17:20:00.000Z", ..ThreadRow::default() };
        fixture.thread(&ThreadRow { branch: Some("feat/discount"), worktree: Some(&tree), ..row("t-kept", "Add a discount helper", "claudeAgent", Some(KEPT)) });
        fixture.thread(&ThreadRow { archived: true, pinned: true, ..row("t-gone", "Casual conversation", "claudeAgent", Some(GONE)) });
        fixture.message("t-gone", "m1", "user", "hi", "2026-09-07T20:35:00.000Z", false);
        fixture.message("t-gone", "m2", "assistant", "Hi! What are we working on today?", "2026-09-07T20:35:01.000Z", false);
        fixture.thread(&row("t-codex", "Round prices", "codex", Some(CODEX)));
        fixture.thread(&ThreadRow { deleted: true, ..row("t-deleted", "Deleted", "claudeAgent", None) });
        fixture.message("t-deleted", "d1", "user", "never comes over", "2026-11-01T00:00:00.000Z", false);
        Home {
            t3: home.join(".t3"),
            claude: claude_import::config_roots(&home, None, &[]),
            codex: vec![codex_home.canonicalize().unwrap()],
            worktree: tree,
            main,
        }
    }

    fn start(core: &Arc<BridgeCore>, tool: &str) -> wire::ImportJob {
        let mut job = new_job();
        job.tool = tool.into();
        save(core, &job, None).unwrap();
        core.import_job_running.store(true, Ordering::SeqCst);
        job
    }

    fn run_t3(core: &Arc<BridgeCore>, home: &Home) -> wire::ImportJob {
        let job = start(core, t3code_import::PROVIDER);
        let (claude, codex) = (home.claude.clone(), home.codex.clone());
        run_with(core, job, std::slice::from_ref(&home.t3), None, &move |source| match source {
            Source::Claude => claude.clone(),
            Source::Codex => codex.clone(),
            _ => Vec::new(),
        });
        status(core).unwrap().unwrap()
    }

    /// Every imported chat: title, harness, archived, workspace path, native key, entries.
    fn chats(core: &BridgeCore) -> Vec<(String, String, bool, Option<String>, Option<String>, i64)> {
        let db = core.db.lock().unwrap();
        let mut statement = db
            .prepare(
                "SELECT s.title, s.harness, s.archived_at IS NOT NULL, w.path,
                    (SELECT native_key FROM import_native_sessions n WHERE n.session_id=s.id),
                    (SELECT COUNT(*) FROM session_entries e WHERE e.session_id=s.id)
                 FROM sessions s LEFT JOIN workspaces w ON w.id=s.workspace_id WHERE s.kind='imported' ORDER BY s.title",
            )
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get::<_, Option<String>>(0)?.unwrap_or_default(), row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    fn threads_come_over_natively_or_as_t3s_copy_under_the_adopted_worktree() {
        let root = tempfile::tempdir().unwrap();
        let home = home(root.path());
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());

        let job = run_t3(&core, &home);

        assert_eq!(job.status, "done", "{job:?}");
        assert_eq!((job.total, job.imported, job.failed, job.workspaces), (3, 3, 0, 1), "{job:?}");
        let tree = home.worktree.to_string_lossy().into_owned();
        assert_eq!(
            chats(&core),
            vec![
                ("Add a discount helper".into(), "claude".into(), false, Some(tree.clone()), Some(format!("claude:{KEPT}")), 2),
                // It ran in the project's folder: filed under that project.
                ("Casual conversation".into(), "claude".into(), true, Some(home.main.to_string_lossy().into_owned()), Some(format!("claude:{GONE}")), 2),
                ("Round prices".into(), "codex".into(), false, None, Some(format!("codex:{CODEX}")), 2),
            ]
        );
        let texts: Vec<(&str, u64)> = job.not_carried.iter().map(|item| (item.text.as_str(), item.count)).collect();
        assert!(texts.contains(&("pins: Bridge has no pinned chats yet", 1)), "{texts:?}");
        assert!(home.worktree.join("README.md").is_file(), "T3 Code's folder is untouched");

        let again = run_t3(&core, &home);
        assert_eq!((again.imported, again.already_in_bridge, again.workspaces, again.failed), (0, 3, 0, 0), "{again:?}");
        assert_eq!(chats(&core).len(), 3);
    }

    #[test]
    fn importing_claude_code_before_or_after_t3_code_makes_one_chat() {
        let root = tempfile::tempdir().unwrap();
        let home = home(root.path());

        // Claude Code first: T3 Code's import finds the chat and gives it T3 Code's title.
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());
        let claude = run_job(&core, &home.claude);
        assert_eq!(claude.imported, 1, "{claude:?}");
        let job = run_t3(&core, &home);
        assert_eq!((job.imported, job.already_in_bridge), (2, 1), "{job:?}");
        let titles: Vec<String> = chats(&core).into_iter().map(|chat| chat.0).collect();
        assert_eq!(titles, vec!["Add a discount helper", "Casual conversation", "Round prices"]);

        // T3 Code first: a Claude Code import after it brings nothing twice.
        let scratch = tempfile::tempdir().unwrap();
        let core = super::core(scratch.path());
        let job = run_t3(&core, &home);
        assert_eq!(job.imported, 3, "{job:?}");
        let claude = run_job(&core, &home.claude);
        assert_eq!((claude.imported, claude.already_in_bridge), (0, 1), "{claude:?}");
        let codex = super::codex::run_codex(&core, &home.codex);
        assert_eq!((codex.imported, codex.already_in_bridge), (0, 1), "{codex:?}");
        assert_eq!(chats(&core).len(), 3);
    }

    #[test]
    fn a_title_renamed_in_bridge_is_kept() {
        let root = tempfile::tempdir().unwrap();
        let home = home(root.path());
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());
        run_job(&core, &home.claude);
        core.db.lock().unwrap().execute("UPDATE sessions SET title='Mine', title_source=NULL WHERE kind='imported'", []).unwrap();
        run_t3(&core, &home);
        let titles: Vec<String> = chats(&core).into_iter().map(|chat| chat.0).collect();
        assert!(titles.contains(&"Mine".to_owned()), "{titles:?}");
    }

    #[test]
    fn only_the_named_claude_sessions_are_listed_and_temp_folder_runs_are_not() {
        let root = tempfile::tempdir().unwrap();
        let claude = root.path().canonicalize().unwrap().join(".claude");
        write(&claude.join("projects/-work-shop/wanted.jsonl"), "{}");
        write(&claude.join("projects/-work-shop/other.jsonl"), "{}");
        write(&claude.join("projects/-work-shop/wanted.txt"), "{}");
        write(&claude.join("projects/-private-var-folders-xy-T-probe/wanted2.jsonl"), "{}");
        write(&claude.join("projects/-work-api/wanted2.jsonl"), "{}");
        // A file where a project folder would be, and a folder named like a transcript.
        write(&claude.join("projects/wanted.jsonl"), "{}");
        fs::create_dir_all(claude.join("projects/-work-x/wanted.jsonl")).unwrap();
        let roots = vec![claude.clone()];
        let work = claude_import::job_work_for_sessions(&roots, &HashSet::from(["wanted", "wanted2"])).unwrap();
        let mut found: Vec<String> = work
            .artifacts
            .iter()
            .map(|artifact| artifact.canonical_source_ref.trim_start_matches(&*claude.to_string_lossy()).to_owned())
            .collect();
        found.sort();
        assert_eq!(found, vec!["/projects/-work-api/wanted2.jsonl", "/projects/-work-shop/wanted.jsonl"]);
        assert!(work.artifacts.iter().all(|artifact| artifact.kind == CandidateKind::Conversation));
    }

    #[test]
    fn only_the_named_codex_threads_are_listed_and_background_runs_are_counted() {
        let root = tempfile::tempdir().unwrap();
        let codex = root.path().canonicalize().unwrap().join(".codex");
        let id = |n: u8| format!("01a1f9f8-cbdc-7a20-a62a-3e214132a14{n}");
        let rollout = |n: u8, cwd: &str| {
            write(&codex.join(format!("sessions/2026/11/20/rollout-2026-11-20T17-19-5{n}-{}.jsonl", id(n))), &[meta(&id(n), cwd, None), said("u", "hi")].join("\n"));
        };
        rollout(1, "/work/shop");
        rollout(2, "/private/var/folders/xy/T/probe");
        rollout(3, "/work/shop");
        let ids = [id(1), id(2)];
        let wanted: HashSet<&str> = ids.iter().map(String::as_str).collect();
        let work = codex_import::job_work_for_threads(&[codex.clone()], &wanted).unwrap();
        assert_eq!(work.artifacts.len(), 1);
        assert!(work.artifacts[0].canonical_source_ref.ends_with(&format!("{}.jsonl", id(1))));
        assert_eq!(work.left_out, 1, "the background run is counted, not listed");
    }

    #[test]
    fn rollout_and_transcript_names_give_their_session_ids() {
        let uuid = "01a1f9f8-cbdc-7a20-a62a-3e214132a143";
        assert_eq!(codex_import::rollout_thread_id(Path::new(&format!("/s/rollout-2026-11-20T17-19-54-{uuid}.jsonl"))).as_deref(), Some(uuid));
        assert_eq!(codex_import::rollout_thread_id(Path::new(&format!("/s/{uuid}.jsonl"))), None, "no dash before the id");
        assert_eq!(codex_import::rollout_thread_id(Path::new("/s/rollout-short.jsonl")), None);
        assert_eq!(codex_import::rollout_thread_id(Path::new(&format!("/s/x{uuid}.jsonl"))), None);
        let artifact = |path: &str| super::artifact(path, 1);
        assert_eq!(native_id(Source::Claude, &artifact("/c/projects/-w/abc-1.jsonl")).as_deref(), Some("abc-1"));
        assert_eq!(native_id(Source::Codex, &artifact(&format!("/x/rollout-2026-11-20T17-19-54-{uuid}.jsonl"))).as_deref(), Some(uuid));
        assert_eq!(native_id(Source::OpenCode, &artifact("/d/opencode.db#ses_1")).as_deref(), Some("ses_1"));
        assert_eq!(native_id(Source::T3Code, &artifact("/d/statev2.sqlite#t")), None);
        assert_eq!(native_id(Source::Conductor, &artifact("/x")), None);
        assert_eq!([Source::Claude, Source::Codex, Source::OpenCode].map(harness_of), [Some("claude"), Some("codex"), Some("opencode")]);
        assert_eq!(harness_of(Source::T3Code), None);
        assert_eq!(harness_of(Source::Conductor), None);
    }

    #[test]
    fn t3_codes_title_and_archived_state_go_on_chats_its_agents_imported() {
        let root = tempfile::tempdir().unwrap();
        let home = home(root.path());
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());
        run_job(&core, &home.claude);
        let (mut store, _) = crate::t3code_import::read(&crate::t3code_import::Paths::at(&home.t3)).unwrap();
        for thread in &mut store.threads {
            if thread.id == "t-kept" {
                thread.archived = true;
            }
        }
        let db = core.db.lock().unwrap();
        let first = crate::t3code_import::overlay(&db, &store).unwrap();
        assert_eq!(first, crate::t3code_import::Overlay { titled: 1, archived: 1 });
        let (title, archived): (String, bool) = db
            .query_row("SELECT title, archived_at IS NOT NULL FROM sessions WHERE kind='imported'", [], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap();
        assert_eq!((title.as_str(), archived), ("Add a discount helper", true));
        assert_eq!(crate::t3code_import::overlay(&db, &store).unwrap(), crate::t3code_import::Overlay::default(), "nothing left to change");
    }

    #[test]
    fn the_overlay_does_nothing_before_any_import() {
        let root = tempfile::tempdir().unwrap();
        let home = home(root.path());
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());
        let (store, _) = crate::t3code_import::read(&crate::t3code_import::Paths::at(&home.t3)).unwrap();
        let db = core.db.lock().unwrap();
        assert_eq!(crate::t3code_import::overlay(&db, &store).unwrap(), crate::t3code_import::Overlay::default());
    }

    #[test]
    fn archived_worktrees_and_pull_request_links_are_reported() {
        let root = tempfile::tempdir().unwrap();
        let home = home(root.path());
        let store = crate::t3code_import::Paths::at(&home.t3).store_v2;
        {
            let fixture = rusqlite::Connection::open(&store).unwrap();
            fixture.execute_batch(&format!(
                "UPDATE orchestration_v2_projection_threads SET payload_json=json_set(payload_json,'$.worktreePath','{}/gone','$.pullRequests',json('[{{\"number\":7}}]')) WHERE thread_id='t-gone';",
                home.t3.display()
            )).unwrap();
        }
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());
        let job = run_t3(&core, &home);
        let texts: Vec<(&str, u64)> = job.not_carried.iter().map(|item| (item.text.as_str(), item.count)).collect();
        assert!(texts.contains(&("worktrees whose threads are all archived in T3 Code", 1)), "{texts:?}");
        assert!(texts.contains(&("pull request links on threads", 1)), "{texts:?}");
    }

    #[test]
    fn a_t3_database_bridge_does_not_know_stops_with_its_version() {
        let root = tempfile::tempdir().unwrap();
        let home = home(root.path());
        let store = crate::t3code_import::Paths::at(&home.t3).store_v2;
        rusqlite::Connection::open(&store).unwrap().execute("INSERT INTO effect_sql_migrations(migration_id,name) VALUES(140,'Future')", []).unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());
        let job = run_t3(&core, &home);
        assert_eq!(job.status, "failed", "{job:?}");
        assert!(job.errors.iter().any(|error| error.contains("newer than Bridge knows (version 140)")), "{:?}", job.errors);
        assert_eq!(chats(&core).len(), 0);
    }
}

mod orca {
    use super::*;
    use crate::import_worktrees::tests::{repo, worktree};
    use crate::orca_import::fixture::{message, worktree_id, ChatRow, Fixture, Handle};

    const REPO: &str = "6e1ad727-cfd9-4417-b1dc-bdcb536e895f";
    /// The Claude Code session a terminal worker ran, captured by Orca.
    const WORKER: &str = "33333333-3333-4333-8333-333333333333";
    /// A Claude Code chat Orca launched with its own account folder.
    const ACCOUNT: &str = "66666666-6666-4666-8666-666666666666";

    struct Home {
        orca: PathBuf,
        claude: Vec<PathBuf>,
        opencode: Vec<PathBuf>,
        login: PathBuf,
        contrib: PathBuf,
        main: PathBuf,
        fixture: Fixture,
        _opencode: crate::opencode_import::fixture::Fixture,
    }

    /// A home where Orca ran, for one repository with two worktrees:
    /// - an OpenCode chat whose session is in OpenCode's store, renamed in Orca;
    /// - a coordinator OpenCode chat whose session is gone (Orca's copy), that
    ///   led a run with a Claude Code terminal worker whose transcript is there;
    /// - a Claude Code chat that never started, one with Pi, and one on
    ///   another machine;
    /// - a Claude Code chat launched with its own account folder, whose
    ///   transcript is only there.
    fn home(root: &Path) -> Home {
        let home = root.canonicalize().unwrap();
        let main = repo(&home.join("code"), "shop");
        let login = worktree(&main, &home.join("orca/workspaces/shop/feat-login"), "feat-login");
        let contrib = worktree(&main, &home.join("orca/workspaces/shop/contrib"), "contrib");
        let store = crate::opencode_import::fixture::Fixture::new(&home.join(".local/share"));
        // Not the test's temp folder: OpenCode's import leaves chats run there out as background runs.
        store.session("ses_a", None, "/work/shop", Some("Repo overview from README.md"), 1_000);
        store.row("ses_a", 1, "user", &crate::opencode_import::fixture::said("what is this repo?"));
        store.row("ses_a", 2, "assistant", &crate::opencode_import::fixture::replied("A shop."));
        write(&home.join(format!(".claude/projects/-shop-contrib/{WORKER}.jsonl")), &session(WORKER, &contrib.to_string_lossy(), "write CONTRIBUTING.md"));
        let account = home.join("orca-accounts/work/claude");
        write(&account.join(format!("projects/-shop-contrib/{ACCOUNT}.jsonl")), &session(ACCOUNT, &contrib.to_string_lossy(), "fix the flaky test"));
        let orca = home.join("Library/Application Support/orca");
        let fixture = Fixture::at(&orca);
        fixture.repos("local-default", &[(REPO, &main)]);
        fixture.worktree_meta(
            "local-default",
            &[(worktree_id(REPO, &main), "", false), (worktree_id(REPO, &login), "Login page", false), (worktree_id(REPO, &contrib), "contrib", false)],
        );
        fixture.chat(&ChatRow {
            id: "chat-a",
            agent: "opencode",
            workspace: worktree_id(REPO, &login),
            title: Some("What is shop?"),
            renamed: true,
            handles: vec![Handle::Acp("opencode", "ses_a")],
            ..ChatRow::default()
        });
        fixture.chat(&ChatRow {
            id: "chat-lead",
            agent: "opencode",
            workspace: worktree_id(REPO, &main),
            title: Some("Orca LICENSE Orchestration"),
            handles: vec![Handle::Acp("opencode", "ses_gone")],
            ..ChatRow::default()
        });
        fixture.items(
            "chat-lead",
            "e1",
            &[message("u1", "user", "Add a LICENSE file", 1_795_419_348_138), message("a1", "assistant", "Started a worker.", 1_795_419_350_000)],
        );
        fixture.chat(&ChatRow { id: "chat-hi", agent: "claude", workspace: worktree_id(REPO, &contrib), title: Some("Say hi"), ..ChatRow::default() });
        fixture.items("chat-hi", "e1", &[message("u1", "user", "Say hi", 1_795_419_429_819)]);
        fixture.chat(&ChatRow { id: "chat-pi", agent: "pi", workspace: worktree_id(REPO, &main), title: Some("Pi"), ..ChatRow::default() });
        fixture.chat(&ChatRow { id: "chat-ssh", agent: "claude", host: "ssh:box", workspace: worktree_id(REPO, &main), title: Some("Remote"), ..ChatRow::default() });
        fixture.chat(&ChatRow {
            id: "chat-account",
            agent: "claude",
            workspace: worktree_id(REPO, &contrib),
            title: Some("Fix the flaky test"),
            handles: vec![Handle::Claude(ACCOUNT)],
            account_home: Some(("CLAUDE_CONFIG_DIR", &account)),
            ..ChatRow::default()
        });
        fixture.run("run_1", Some("chat-lead"));
        fixture.terminal_worker("run_1", "ctx_1", "claude", Some(WORKER));
        fixture.terminal_worker("run_1", "ctx_2", "opencode", None);
        Home {
            claude: claude_import::config_roots(&home, None, &[]),
            opencode: vec![store.home.clone()],
            orca,
            login,
            contrib,
            main,
            fixture,
            _opencode: store,
        }
    }

    fn run_orca(core: &Arc<BridgeCore>, home: &Home) -> wire::ImportJob {
        let mut job = new_job();
        job.tool = orca_import::PROVIDER.into();
        save(core, &job, None).unwrap();
        core.import_job_running.store(true, Ordering::SeqCst);
        let (claude, opencode) = (home.claude.clone(), home.opencode.clone());
        run_with(core, job, std::slice::from_ref(&home.orca), None, &move |source| match source {
            Source::Claude => claude.clone(),
            Source::OpenCode => opencode.clone(),
            _ => Vec::new(),
        });
        status(core).unwrap().unwrap()
    }

    /// Every imported chat: title, harness, workspace path, native key, parent's title.
    fn chats(core: &BridgeCore) -> Vec<(String, String, Option<String>, Option<String>, Option<String>)> {
        let db = core.db.lock().unwrap();
        let mut statement = db
            .prepare(
                "SELECT s.title, s.harness, w.path,
                    (SELECT native_key FROM import_native_sessions n WHERE n.session_id=s.id),
                    (SELECT p.title FROM sessions p WHERE p.id=s.parent_session_id)
                 FROM sessions s LEFT JOIN workspaces w ON w.id=s.workspace_id WHERE s.kind='imported' ORDER BY s.title",
            )
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get::<_, Option<String>>(0)?.unwrap_or_default(), row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    fn chats_come_over_natively_or_as_orcas_copy_with_workers_under_their_lead() {
        let root = tempfile::tempdir().unwrap();
        let home = home(root.path());
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());

        let job = run_orca(&core, &home);

        assert_eq!(job.status, "done", "{job:?}");
        // Three read natively and two of Orca's copies: five, not six.
        assert_eq!((job.total, job.imported, job.failed, job.workspaces), (5, 5, 0, 2), "{job:?}");
        let path = |path: &Path| Some(path.to_string_lossy().into_owned());
        let lead = "Orca LICENSE Orchestration".to_owned();
        assert_eq!(
            chats(&core),
            vec![
                // Read from the account folder Orca launched it with, given Orca's title.
                ("Fix the flaky test".into(), "claude".into(), path(&home.contrib), Some(format!("claude:{ACCOUNT}")), None),
                // Orca's copy, in the main checkout: filed under its project.
                (lead.clone(), "opencode".into(), path(&home.main), Some("opencode:ses_gone".into()), None),
                ("Say hi".into(), "claude".into(), path(&home.contrib), None, None),
                // Read by OpenCode's own reader, with Orca's title.
                ("What is shop?".into(), "opencode".into(), None, Some("opencode:ses_a".into()), None),
                // The terminal worker's own transcript, under the chat that led it, in its worktree.
                ("write CONTRIBUTING.md".into(), "claude".into(), path(&home.contrib), Some(format!("claude:{WORKER}")), Some(lead)),
            ]
        );
        let texts: Vec<(&str, u64)> = job.not_carried.iter().map(|item| (item.text.as_str(), item.count)).collect();
        assert!(texts.contains(&("chats with pi, which Bridge does not run", 1)), "{texts:?}");
        assert!(texts.contains(&("chats on another machine (SSH or WSL)", 1)), "{texts:?}");
        assert!(texts.iter().any(|(text, count)| text.starts_with("workers' place") && *count == 1), "{texts:?}");
        assert!(home.login.join("README.md").is_file() && home.contrib.join("README.md").is_file(), "Orca's folders are untouched");

        let mut events = core.events.subscribe();
        let again = run_orca(&core, &home);
        assert_eq!((again.imported, again.already_in_bridge, again.workspaces, again.failed), (0, 5, 0, 0), "{again:?}");
        assert_eq!(chats(&core).len(), 5);
        assert!(!changed(&mut events), "a run that changed nothing tells no one it did");
    }

    #[test]
    fn importing_opencode_or_claude_code_after_orca_brings_nothing_twice() {
        let root = tempfile::tempdir().unwrap();
        let home = home(root.path());
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());
        let job = run_orca(&core, &home);
        assert_eq!(job.imported, 5, "{job:?}");
        let opencode = super::opencode::run_opencode(&core, &home.opencode);
        assert_eq!((opencode.imported, opencode.already_in_bridge), (0, 1), "{opencode:?}");
        let claude = run_job(&core, &home.claude);
        assert_eq!((claude.imported, claude.already_in_bridge), (0, 1), "{claude:?}");
        assert_eq!(chats(&core).len(), 5);
    }

    #[test]
    fn orca_after_its_agents_links_the_worker_and_titles_the_chat() {
        let root = tempfile::tempdir().unwrap();
        let home = home(root.path());
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());
        run_job(&core, &home.claude);
        super::opencode::run_opencode(&core, &home.opencode);
        let job = run_orca(&core, &home);
        assert_eq!((job.imported, job.already_in_bridge), (3, 2), "{job:?}");
        let chats = chats(&core);
        assert!(chats.iter().any(|chat| chat.0 == "What is shop?"), "{chats:?}");
        let worker = chats.iter().find(|chat| chat.3.as_deref() == Some(&format!("claude:{WORKER}"))).unwrap();
        assert_eq!(worker.4.as_deref(), Some("Orca LICENSE Orchestration"), "{chats:?}");
        let depth: i64 = core
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT s.depth FROM sessions s JOIN import_native_sessions n ON n.session_id=s.id WHERE n.native_key=?1",
                params![format!("claude:{WORKER}")],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(depth, 1);
    }

    fn depth_and_parent(core: &BridgeCore, key: &str) -> (String, i64, Option<String>) {
        core.db.lock().unwrap().query_row(
            "SELECT s.id, COALESCE(s.depth,0), s.parent_session_id FROM sessions s JOIN import_native_sessions n ON n.session_id=s.id WHERE n.native_key=?1",
            params![key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap()
    }

    #[test]
    fn a_linked_workers_own_subagents_move_down_with_it() {
        let root = tempfile::tempdir().unwrap();
        let home = home(root.path());
        write(
            &home.claude[0].join(format!("projects/-shop-contrib/{WORKER}/subagents/agent-a1.jsonl")),
            &session(WORKER, &home.contrib.to_string_lossy(), "check the file"),
        );
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());
        run_orca(&core, &home);
        let (worker, depth, parent) = depth_and_parent(&core, &format!("claude:{WORKER}"));
        assert_eq!(depth, 1);
        assert!(parent.is_some());
        let child: i64 = core.db.lock().unwrap().query_row("SELECT COALESCE(depth,0) FROM sessions WHERE parent_session_id=?1", params![worker], |row| row.get(0)).unwrap();
        assert_eq!(child, 2, "the worker's subagent sits under it");
    }

    #[test]
    fn a_worker_already_under_another_chat_keeps_its_place() {
        let root = tempfile::tempdir().unwrap();
        let home = home(root.path());
        write(
            &home.claude[0].join(format!("projects/-shop-contrib/{WORKER}/subagents/agent-a1.jsonl")),
            &session(WORKER, &home.contrib.to_string_lossy(), "check the file"),
        );
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());
        run_orca(&core, &home);
        let (worker, _, lead) = depth_and_parent(&core, &format!("claude:{WORKER}"));
        let other: String = core.db.lock().unwrap().query_row("SELECT id FROM sessions WHERE title='Say hi'", [], |row| row.get(0)).unwrap();
        {
            // Placed under another chat since, with its subagent under it, and its old lead deeper.
            let db = core.db.lock().unwrap();
            db.execute("UPDATE sessions SET parent_session_id=?1, depth=7 WHERE id=?2", params![other, worker]).unwrap();
            db.execute("UPDATE sessions SET depth=8 WHERE parent_session_id=?1", params![worker]).unwrap();
            db.execute("UPDATE sessions SET depth=3 WHERE id=?1", params![lead.unwrap()]).unwrap();
        }
        run_orca(&core, &home);
        let (_, depth, parent) = depth_and_parent(&core, &format!("claude:{WORKER}"));
        assert_eq!((depth, parent.as_deref()), (7, Some(other.as_str())), "a chat already placed is never moved");
        let child: i64 = core.db.lock().unwrap().query_row("SELECT depth FROM sessions WHERE parent_session_id=?1", params![worker], |row| row.get(0)).unwrap();
        assert_eq!(child, 8, "nor are its subagents");
    }

    #[test]
    fn a_run_that_only_retitles_says_the_state_changed() {
        let root = tempfile::tempdir().unwrap();
        let home = home(root.path());
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());
        run_orca(&core, &home);
        home.fixture.chat(&ChatRow {
            id: "chat-a",
            agent: "opencode",
            workspace: worktree_id(REPO, &home.login),
            title: Some("What is this shop?"),
            renamed: true,
            handles: vec![Handle::Acp("opencode", "ses_a")],
            ..ChatRow::default()
        });
        let mut events = core.events.subscribe();
        let job = run_orca(&core, &home);
        assert_eq!(job.imported, 0, "{job:?}");
        assert!(changed(&mut events), "Orca's new title changed a chat");

        // Archived in Orca since: the chat its agent's import brought is archived.
        home.fixture.worktree_meta("local-default", &[(worktree_id(REPO, &home.login), "Login page", true)]);
        let mut events = core.events.subscribe();
        run_orca(&core, &home);
        assert!(changed(&mut events), "archiving in Orca archived its chat");
        let archived: bool = core.db.lock().unwrap().query_row("SELECT archived_at IS NOT NULL FROM sessions s JOIN import_native_sessions n ON n.session_id=s.id WHERE n.native_key='opencode:ses_a'", [], |row| row.get(0)).unwrap();
        assert!(archived);

        // A worker that lost its place is put back under its lead.
        let (worker, _, _) = depth_and_parent(&core, &format!("claude:{WORKER}"));
        core.db.lock().unwrap().execute("UPDATE sessions SET parent_session_id=NULL, depth=0 WHERE id=?1", params![worker]).unwrap();
        let mut events = core.events.subscribe();
        run_orca(&core, &home);
        assert!(changed(&mut events), "linking a worker changed the chats");
        assert!(depth_and_parent(&core, &format!("claude:{WORKER}")).2.is_some());
    }

    #[test]
    fn an_archived_worktree_is_left_out_and_its_chats_come_over_archived() {
        let root = tempfile::tempdir().unwrap();
        let home = home(root.path());
        home.fixture.worktree_meta(
            "local-default",
            &[(worktree_id(REPO, &home.login), "Login page", false), (worktree_id(REPO, &home.contrib), "contrib", true)],
        );
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());
        let job = run_orca(&core, &home);
        assert_eq!(job.workspaces, 1, "{job:?}");
        let texts: Vec<(&str, u64)> = job.not_carried.iter().map(|item| (item.text.as_str(), item.count)).collect();
        assert!(texts.contains(&("worktrees archived in Orca", 1)), "{texts:?}");
        let archived: i64 = core
            .db
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM sessions WHERE kind='imported' AND title='Say hi' AND archived_at IS NOT NULL", [], |row| row.get(0))
            .unwrap();
        assert_eq!(archived, 1);
    }

    #[test]
    fn a_newer_chat_database_fails_the_job_and_imports_nothing() {
        let root = tempfile::tempdir().unwrap();
        let home = home(root.path());
        home.fixture.chats.execute_batch("PRAGMA user_version = 9").unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());
        let job = run_orca(&core, &home);
        assert_eq!(job.status, "failed", "{job:?}");
        assert!(job.errors.iter().any(|error| error.contains("version 9")), "{:?}", job.errors);
        assert_eq!(chats(&core).len(), 0);
    }

    #[test]
    fn orca_is_a_source_with_its_label_and_no_listing_of_its_own() {
        assert_eq!(Source::of("orca"), Some(Source::Orca));
        assert_eq!(Source::Orca.label(), "Orca");
        assert_eq!(native_id(Source::Orca, &super::artifact("/d/agent-session-domain.db#c", 1)), None);
        assert_eq!(harness_of(Source::Orca), None);
        let work = Source::Orca.work(&[PathBuf::from("/o")]).unwrap();
        assert!(work.artifacts.is_empty());
        let artifact = super::artifact("/d/agent-session-domain.db#c", 1);
        assert_eq!(Source::Orca.key(&artifact), "/d/agent-session-domain.db#c");
    }
}

mod superset {
    use super::*;
    use crate::import_worktrees::tests::{repo, worktree};
    use crate::superset_import::fixture::{agent, user, ChatRow, Fixture, WorkspaceRow, CANARY};

    /// A Claude Code session a terminal ran, whose transcript is there.
    const KEPT: &str = "33333333-3333-4333-8333-333333333333";
    /// One whose transcript Claude Code deleted.
    const GONE: &str = "44444444-4444-4444-8444-444444444444";
    /// A Claude Code terminal session whose transcript is in another Claude
    /// Code folder, which only its `transcript_path` names.
    const ELSEWHERE: &str = "77777777-7777-4777-8777-777777777777";

    struct Home {
        paradev: PathBuf,
        claude: Vec<PathBuf>,
        opencode: Vec<PathBuf>,
        login: PathBuf,
        fixture: Fixture,
        _opencode: crate::opencode_import::fixture::Fixture,
    }

    /// A home where Superset ran, for one repository:
    /// - a worktree workspace with an OpenCode chat whose session OpenCode
    ///   still has, a Claude Code chat whose session is gone (Superset's
    ///   copy), and a Claude Code terminal session whose transcript is there;
    /// - an archived workspace with a chat;
    /// - a terminal session whose transcript is gone, one that never
    ///   reported its session, and one with an agent Bridge does not run;
    /// - a chat with Pi, and a chat and a terminal in a deleted project;
    /// - an agent configuration with an API key.
    fn home(root: &Path) -> Home {
        let home = root.canonicalize().unwrap();
        let main = repo(&home.join("code"), "shop");
        let paradev = home.join(".paradev");
        let login = worktree(&main, &paradev.join("worktrees/shop/feat-login"), "feat-login");
        let store = crate::opencode_import::fixture::Fixture::new(&home.join(".local/share"));
        // Not the test's temp folder: OpenCode's import leaves chats run there out as background runs.
        store.session("ses_a", None, "/work/shop", Some("Repo overview from README.md"), 1_000);
        store.row("ses_a", 1, "user", &crate::opencode_import::fixture::said("what is this repo?"));
        store.row("ses_a", 2, "assistant", &crate::opencode_import::fixture::replied("A shop."));
        write(&home.join(format!(".claude/projects/-shop-login/{KEPT}.jsonl")), &session(KEPT, &login.to_string_lossy(), "write CONTRIBUTING.md"));
        let elsewhere = home.join("claude-work").join(format!("projects/-shop-login/{ELSEWHERE}.jsonl"));
        write(&elsewhere, &session(ELSEWHERE, &login.to_string_lossy(), "tidy the imports"));
        let fixture = Fixture::newest(&paradev);
        fixture.project("p1", &main, "shop");
        fixture.workspace(&WorkspaceRow { id: "w-login", project: Some("p1"), path: &login, branch: "feat-login", name: "Login page", kind: "worktree", archived: false });
        fixture.workspace(&WorkspaceRow { id: "w-old", project: Some("p1"), path: &paradev.join("worktrees/shop/old-spike"), branch: "old-spike", name: "Old spike", kind: "worktree", archived: true });
        fixture.workspace(&WorkspaceRow { id: "w-local", project: Some("p1"), path: &main, branch: "main", name: "shop", kind: "local", archived: false });
        fixture.chat(&ChatRow { id: "c-open", workspace: "w-login", native: Some("ses_a"), ..ChatRow::default() });
        fixture.chat(&ChatRow { id: "c-claude", workspace: "w-login", harness: "claude-acp", native: Some(GONE), ..ChatRow::default() });
        fixture.events("c-claude", "e1", &[user("u1", "Say hi"), agent("a1", "Hi! What are we working on?")]);
        fixture.chat(&ChatRow { id: "c-old", workspace: "w-old", harness: "claude-acp", ..ChatRow::default() });
        fixture.events("c-old", "e1", &[user("u1", "An old spike"), agent("a1", "Archived since.")]);
        fixture.binding("t-kept", "w-login", "claude", Some(KEPT), None);
        fixture.binding("t-gone", "w-login", "claude", Some("55555555-5555-4555-8555-555555555555"), None);
        fixture.binding("t-silent", "w-login", "opencode", None, None);
        fixture.binding("t-amp", "w-login", "amp", Some("amp-1"), None);
        fixture.binding("t-elsewhere", "w-login", "claude", Some(ELSEWHERE), Some(&elsewhere));
        fixture.chat(&ChatRow { id: "c-pi", workspace: "w-login", harness: "pi-acp", native: Some("pi-1"), ..ChatRow::default() });
        fixture.project("p-gone", &home.join("code/gone"), "gone");
        fixture.delete_project("p-gone");
        fixture.workspace(&WorkspaceRow { id: "w-gone", project: Some("p-gone"), path: &paradev.join("worktrees/gone/x"), branch: "x", name: "Gone", kind: "worktree", archived: false });
        fixture.chat(&ChatRow { id: "c-gone", workspace: "w-gone", harness: "claude-acp", ..ChatRow::default() });
        fixture.events("c-gone", "e1", &[user("u1", "never comes over")]);
        fixture.binding("t-gone-project", "w-gone", "claude", Some("88888888-8888-4888-8888-888888888888"), None);
        fixture.tag("w-login", "auth");
        fixture.config("cfg-key", Some("claude"), "Claude (work key)", &json!({ "ANTHROPIC_API_KEY": CANARY }));
        Home {
            claude: claude_import::config_roots(&home, None, &[]),
            opencode: vec![store.home.clone()],
            paradev,
            login,
            fixture,
            _opencode: store,
        }
    }

    fn run_superset(core: &Arc<BridgeCore>, home: &Home) -> wire::ImportJob {
        let mut job = new_job();
        job.tool = superset_import::PROVIDER.into();
        save(core, &job, None).unwrap();
        core.import_job_running.store(true, Ordering::SeqCst);
        let (claude, opencode) = (home.claude.clone(), home.opencode.clone());
        run_with(core, job, std::slice::from_ref(&home.paradev), None, &move |source| match source {
            Source::Claude => claude.clone(),
            Source::OpenCode => opencode.clone(),
            _ => Vec::new(),
        });
        status(core).unwrap().unwrap()
    }

    /// Every imported chat: title, harness, archived, workspace path, native key.
    fn chats(core: &BridgeCore) -> Vec<(String, String, bool, Option<String>, Option<String>)> {
        let db = core.db.lock().unwrap();
        let mut statement = db
            .prepare(
                "SELECT s.title, s.harness, s.archived_at IS NOT NULL, w.path,
                    (SELECT native_key FROM import_native_sessions n WHERE n.session_id=s.id)
                 FROM sessions s LEFT JOIN workspaces w ON w.id=s.workspace_id WHERE s.kind='imported' ORDER BY s.title",
            )
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get::<_, Option<String>>(0)?.unwrap_or_default(), row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    fn setup_candidates(core: &BridgeCore) -> Vec<(String, String, String)> {
        let db = core.db.lock().unwrap();
        let mut statement = db.prepare("SELECT candidate_kind, title, activation_state FROM import_setup_candidates ORDER BY title").unwrap();
        statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap().map(Result::unwrap).collect()
    }

    #[test]
    fn chats_and_terminal_sessions_come_over_natively_or_as_supersets_copy_under_the_workspace() {
        let root = tempfile::tempdir().unwrap();
        let home = home(root.path());
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());

        let job = run_superset(&core, &home);

        assert_eq!(job.status, "done", "{job:?}");
        // Three read natively, two of Superset's copies and the configuration.
        assert_eq!((job.total, job.imported, job.failed, job.workspaces), (6, 6, 0, 1), "{job:?}");
        let tree = Some(home.login.to_string_lossy().into_owned());
        assert_eq!(
            chats(&core),
            vec![
                ("An old spike".into(), "claude".into(), true, None, None),
                ("Repo overview from README.md".into(), "opencode".into(), false, None, Some("opencode:ses_a".into())),
                ("Say hi".into(), "claude".into(), false, tree.clone(), Some(format!("claude:{GONE}"))),
                // Read from the folder its transcript path names.
                ("tidy the imports".into(), "claude".into(), false, tree.clone(), Some(format!("claude:{ELSEWHERE}"))),
                ("write CONTRIBUTING.md".into(), "claude".into(), false, tree, Some(format!("claude:{KEPT}"))),
            ]
        );
        assert_eq!(setup_candidates(&core), vec![("provider_instance".into(), "Claude (work key)".into(), "disabled".into())]);
        let payload: String = core.db.lock().unwrap().query_row("SELECT payload FROM import_setup_candidates", [], |row| row.get(0)).unwrap();
        assert!(payload.contains("ANTHROPIC_API_KEY") && !payload.contains(CANARY), "{payload}");
        let texts: Vec<(&str, u64)> = job.not_carried.iter().map(|item| (item.text.as_str(), item.count)).collect();
        for want in [
            ("workspaces archived in Superset", 1),
            ("terminal sessions with amp, which Bridge does not run", 1),
            ("terminal agents that never reported their session", 1),
            ("terminal sessions whose transcript is gone (Superset keeps no copy of them)", 1),
            ("workspace tags: Bridge has no tags yet", 1),
            ("chats with pi-acp, which Bridge does not run", 1),
            ("chats in projects deleted in Superset", 1),
            ("terminal sessions in projects deleted in Superset", 1),
        ] {
            assert!(texts.contains(&want), "{want:?} in {texts:?}");
        }
        assert_eq!(texts.len(), 8, "nothing else, and never a zero: {texts:?}");
        assert!(home.login.join("README.md").is_file(), "Superset's folder is untouched");

        let mut events = core.events.subscribe();
        let again = run_superset(&core, &home);
        assert_eq!((again.imported, again.already_in_bridge, again.workspaces, again.failed), (0, 6, 0, 0), "{again:?}");
        assert_eq!(chats(&core).len(), 5);
        assert!(!changed(&mut events), "a run that changed nothing tells no one it did");
        assert_eq!(setup_candidates(&core).len(), 1);
    }

    #[test]
    fn importing_opencode_or_claude_code_after_superset_brings_nothing_twice() {
        let root = tempfile::tempdir().unwrap();
        let home = home(root.path());
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());
        assert_eq!(run_superset(&core, &home).imported, 6);
        let opencode = super::opencode::run_opencode(&core, &home.opencode);
        assert_eq!((opencode.imported, opencode.already_in_bridge), (0, 1), "{opencode:?}");
        let claude = run_job(&core, &home.claude);
        assert_eq!((claude.imported, claude.already_in_bridge), (0, 1), "{claude:?}");
        assert_eq!(chats(&core).len(), 5);
    }

    #[test]
    fn supersets_title_goes_on_chats_its_agents_imported_unless_renamed_in_bridge() {
        let root = tempfile::tempdir().unwrap();
        let home = home(root.path());
        home.fixture.chat_conn().execute("UPDATE chat_sessions_local SET title = 'Explain the shop' WHERE session_id = 'c-open'", []).unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());
        super::opencode::run_opencode(&core, &home.opencode);
        run_superset(&core, &home);
        let titles: Vec<String> = chats(&core).into_iter().map(|chat| chat.0).collect();
        assert!(titles.contains(&"Explain the shop".to_owned()), "{titles:?}");

        let scratch = tempfile::tempdir().unwrap();
        let core = super::core(scratch.path());
        super::opencode::run_opencode(&core, &home.opencode);
        core.db.lock().unwrap().execute("UPDATE sessions SET title='Mine', title_source=NULL WHERE kind='imported'", []).unwrap();
        run_superset(&core, &home);
        let titles: Vec<String> = chats(&core).into_iter().map(|chat| chat.0).collect();
        assert!(titles.contains(&"Mine".to_owned()) && !titles.contains(&"Explain the shop".to_owned()), "{titles:?}");
    }

    #[test]
    fn a_run_that_only_retitles_or_archives_says_the_state_changed() {
        let root = tempfile::tempdir().unwrap();
        let home = home(root.path());
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());
        run_superset(&core, &home);
        home.fixture.chat_conn().execute("UPDATE chat_sessions_local SET title = 'Explain the shop' WHERE session_id = 'c-open'", []).unwrap();
        let mut events = core.events.subscribe();
        let job = run_superset(&core, &home);
        assert_eq!(job.imported, 0, "{job:?}");
        assert!(changed(&mut events), "Superset's new title changed a chat");
        home.fixture.host.execute("UPDATE workspaces SET archived_at = 5 WHERE id = 'w-login'", []).unwrap();
        let mut events = core.events.subscribe();
        run_superset(&core, &home);
        assert!(changed(&mut events), "archiving in Superset archived its chats");
    }

    #[test]
    fn a_newer_host_database_fails_the_job_and_imports_nothing() {
        let root = tempfile::tempdir().unwrap();
        let home = home(root.path());
        home.fixture.host.execute("INSERT INTO __drizzle_migrations(hash, created_at) VALUES('h', ?1)", [superset_import::store::HOST_NEWEST + 1]).unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let core = core(scratch.path());
        let job = run_superset(&core, &home);
        assert_eq!(job.status, "failed", "{job:?}");
        assert!(job.errors.iter().any(|error| error.contains("newer")), "{:?}", job.errors);
        assert_eq!(chats(&core).len(), 0);
    }

    #[test]
    fn superset_is_a_source_with_its_label_and_no_listing_of_its_own() {
        assert_eq!(Source::of("superset"), Some(Source::Superset));
        assert_eq!(Source::Superset.label(), "Superset");
        assert_eq!(native_id(Source::Superset, &super::artifact("/h/host/o/chat.db#c", 1)), None);
        assert_eq!(harness_of(Source::Superset), None);
        assert!(Source::Superset.work(&[PathBuf::from("/h")]).unwrap().artifacts.is_empty());
        let artifact = super::artifact("/h/host/o/chat.db#c", 1);
        assert_eq!(Source::Superset.key(&artifact), "/h/host/o/chat.db#c");
    }
}

/// The T3 Code job against this Mac's real T3 Code store (`$T3CODE_HOME`,
/// else `~/.t3`) and its real Claude Code, Codex and OpenCode folders, into a
/// scratch database. Prints aggregate numbers only.
#[test]
#[ignore = "reads the real ~/.t3; run by hand to measure"]
fn measure_the_t3code_job_on_this_machine() {
    let host = crate::provider_accounts::HostHomes::from_env();
    let paths = crate::t3code_import::Paths::from_env(&host.home);
    let (store, path) = crate::t3code_import::read(&paths).unwrap();
    let live = crate::t3code_import::chats(&store).count();
    println!("MEASURE store={} version={} era={:?} threads={} live_chats={}", path.display(), store.version, store.era, store.threads.len(), live);
    let scratch = tempfile::tempdir().unwrap();
    let core = core(scratch.path());
    let mut job = new_job();
    job.tool = crate::t3code_import::PROVIDER.into();
    save(&core, &job, None).unwrap();
    core.import_job_running.store(true, Ordering::SeqCst);
    let started = Instant::now();
    run(&core, job, &[paths.base.clone()], None);
    let job = status(&core).unwrap().unwrap();
    println!(
        "MEASURE job_secs={:.2} status={} total={} imported={} already={} failed={} workspaces={} not_carried={:?} errors={:?}",
        started.elapsed().as_secs_f64(), job.status, job.total, job.imported, job.already_in_bridge, job.failed, job.workspaces,
        job.not_carried.iter().map(|item| (item.text.clone(), item.count)).collect::<Vec<_>>(), job.errors
    );
    let db = core.db.lock().unwrap();
    let mut statement = db
        .prepare("SELECT harness, archived_at IS NOT NULL, workspace_id IS NOT NULL, (SELECT COUNT(*) FROM session_entries e WHERE e.session_id=s.id), (SELECT native_key IS NOT NULL FROM import_native_sessions n WHERE n.session_id=s.id) FROM sessions s WHERE kind='imported' ORDER BY started_at")
        .unwrap();
    for row in statement
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?, row.get::<_, bool>(2)?, row.get::<_, i64>(3)?, row.get::<_, Option<bool>>(4)?)))
        .unwrap()
    {
        println!("MEASURE chat {:?}", row.unwrap());
    }
}

/// For the import fidelity eval (`T3_EVAL_OUT=<file>`): the T3 Code job
/// against this Mac's T3 Code store (`$T3CODE_HOME`, else `~/.t3`) into a
/// scratch database, then every imported chat's title, project, source and
/// user and assistant texts, as JSON.
#[test]
#[ignore = "reads the real T3 Code store; run by hand for the eval"]
fn dump_the_t3code_import_for_the_eval() {
    let out = std::env::var("T3_EVAL_OUT").expect("set T3_EVAL_OUT");
    let host = crate::provider_accounts::HostHomes::from_env();
    let paths = crate::t3code_import::Paths::from_env(&host.home);
    let (store, _) = crate::t3code_import::read(&paths).unwrap();
    let project_of = |key: Option<&str>, title: &str| -> Option<String> {
        store
            .threads
            .iter()
            .find(|thread| key.is_some_and(|key| thread.native.as_ref().map(|native| native.key()).as_deref() == Some(key)) || thread.title.as_deref() == Some(title))
            .and_then(|thread| store.projects.get(&thread.project_id))
            .map(|project| project.root.to_string_lossy().into_owned())
    };
    let scratch = tempfile::tempdir().unwrap();
    let core = core(scratch.path());
    let mut job = new_job();
    job.tool = crate::t3code_import::PROVIDER.into();
    save(&core, &job, None).unwrap();
    core.import_job_running.store(true, Ordering::SeqCst);
    run(&core, job, &[paths.base.clone()], None);
    let job = status(&core).unwrap().unwrap();
    println!("EVAL job status={} imported={} failed={}", job.status, job.imported, job.failed);
    let db = core.db.lock().unwrap();
    let mut sessions = db
        .prepare("SELECT s.id, s.title, (SELECT native_key FROM import_native_sessions n WHERE n.session_id=s.id) FROM sessions s WHERE kind='imported' AND parent_session_id IS NULL")
        .unwrap();
    let rows: Vec<(String, String, Option<String>)> = sessions
        .query_map([], |row| Ok((row.get(0)?, row.get::<_, Option<String>>(1)?.unwrap_or_default(), row.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut chats = Vec::new();
    for (id, title, key) in rows {
        let mut entries = db.prepare("SELECT kind, payload FROM session_entries WHERE session_id=?1 ORDER BY sequence").unwrap();
        let mut source = String::new();
        let texts: Vec<serde_json::Value> = entries
            .query_map([&id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .filter_map(|(kind, payload)| {
                let payload: serde_json::Value = serde_json::from_str(&payload).ok()?;
                if source.is_empty() {
                    source = payload["importSource"].as_str().unwrap_or_default().to_owned();
                }
                let role = match kind.as_str() {
                    "user.message" => "user",
                    "assistant.message" => "assistant",
                    _ => return None,
                };
                Some(json!({ "role": role, "text": payload["text"].as_str().unwrap_or_default() }))
            })
            .collect();
        chats.push(json!({ "title": title, "project": project_of(key.as_deref(), &title), "source": source, "nativeKey": key, "entries": texts }));
    }
    fs::write(&out, serde_json::to_string_pretty(&chats).unwrap()).unwrap();
    println!("EVAL wrote {} chats to {out}", chats.len());
}

/// The Codex job against this Mac's real Codex home, into a scratch
/// database. Prints aggregate numbers only.
#[test]
#[ignore = "reads the real ~/.codex; run by hand to measure"]
fn measure_the_codex_job_on_this_machine() {
    let roots = crate::codex_import::machine_codex_homes();
    let started = Instant::now();
    let work = crate::codex_import::job_work(&roots).unwrap();
    println!("MEASURE work_list_secs={:.1} files={} left_out={}", started.elapsed().as_secs_f64(), work.artifacts.len(), work.left_out);
    let scratch = tempfile::tempdir().unwrap();
    let core = core(scratch.path());
    let started = Instant::now();
    let job = codex::run_codex(&core, &roots);
    println!(
        "MEASURE job_secs={:.1} status={} processed={} imported={} already={} failed={} left_out={} errors={}",
        started.elapsed().as_secs_f64(), job.status, job.processed, job.imported, job.already_in_bridge, job.failed, job.left_out, job.errors.len()
    );
    for item in &job.not_carried {
        println!("MEASURE not_carried: {} x {}", item.count, item.text.chars().take(120).collect::<String>());
    }
    for error in &job.errors {
        println!("MEASURE error: {}", error.chars().take(200).collect::<String>());
    }
    let tools: i64 = core.db.lock().unwrap().query_row("SELECT COUNT(*) FROM session_entries WHERE kind='tool.completed'", [], |row| row.get(0)).unwrap();
    let children: i64 = core.db.lock().unwrap().query_row("SELECT COUNT(*) FROM sessions WHERE kind='imported' AND parent_session_id IS NOT NULL", [], |row| row.get(0)).unwrap();
    println!("MEASURE tool_rows={tools} child_chats={children}");
}

/// What the removed Review step cost: Claude Code discovery over this Mac's
/// real store, as Review ran it. Prints aggregate numbers only.
#[test]
#[ignore = "reads the real ~/.claude; run by hand to measure"]
fn measure_claude_discovery_on_this_machine() {
    use crate::external_import::{DiscoveryRequest, ExternalHarnessImporter};
    let roots = claude_import::machine_config_roots();
    let request = DiscoveryRequest {
        provider: claude_import::PROVIDER.into(),
        approved_roots: roots.iter().map(|r| r.to_string_lossy().into_owned()).collect(),
        selected_export: None, source_version: None, schema_version: None,
        format_versions: std::collections::BTreeMap::new(),
    };
    let started = Instant::now();
    let discovery = ClaudeCodeImporter.discover(&request).unwrap();
    let bytes = serde_json::to_vec(&discovery).unwrap().len();
    println!("MEASURE discover_secs={:.1} artifacts={} json_mb={:.1}", started.elapsed().as_secs_f64(), discovery.artifacts.len(), bytes as f64 / 1e6);
}

/// Each tool's section shows its own last import: a Codex import after a
/// Claude Code one does not hide the Claude Code result.
#[test]
fn the_latest_import_of_each_tool_is_kept() {
    let scratch = tempfile::tempdir().unwrap();
    let core = core(scratch.path());
    let mut first = new_job();
    first.status = "done".into();
    save(&core, &first, None).unwrap();
    let mut codex = new_job();
    codex.tool = crate::codex_import::PROVIDER.into();
    save(&core, &codex, None).unwrap();
    let mut second = new_job();
    second.status = "done".into();
    save(&core, &second, None).unwrap();

    let jobs = status_by_tool(&core).unwrap();

    let ids: Vec<(&str, &str)> = jobs.iter().map(|job| (job.tool.as_str(), job.job_id.as_str())).collect();
    assert_eq!(ids, vec![(claude_import::PROVIDER, second.job_id.as_str()), (crate::codex_import::PROVIDER, codex.job_id.as_str())]);
}

/// Conductor at scale: 50 repos with 40 real worktrees each and a database
/// from the real schema, adopted through the job. Prints time only.
#[test]
#[ignore = "builds 2,000 real worktrees; run by hand to measure"]
fn measure_conductor_at_scale() {
    use crate::conductor_import::fixture::{Fixture, Row};
    use crate::import_worktrees::tests::{git, repo};
    let root = tempfile::tempdir().unwrap();
    let home = root.path().canonicalize().unwrap();
    let fixture = Fixture::at(&crate::conductor_import::Paths::for_home(&home).store, 181);
    let started = Instant::now();
    for r in 0..50 {
        let name = format!("repo{r:02}");
        let main = repo(&home.join("code"), &name);
        fixture.repo(&format!("r{r}"), &name, &main, 3);
        for w in 0..40 {
            let directory = format!("ws{w:02}");
            let path = home.join("conductor/workspaces").join(&name).join(&directory);
            git(&main, &["worktree", "add", "-q", "-b", &format!("{name}-{directory}"), &path.to_string_lossy()]);
            let key = format!("k{r}-{w}");
            fixture.workspace(&Row { key: &key, repo: &format!("r{r}"), directory: &directory, name: Some("Some work"), branch: Some("b"), state: if w % 10 == 0 { "archived" } else { "active" }, path: None });
        }
    }
    println!("MEASURE setup_secs={:.1}", started.elapsed().as_secs_f64());
    let scratch = tempfile::tempdir().unwrap();
    let core = core(scratch.path());
    let started = Instant::now();
    let mut job = new_job();
    job.tool = crate::conductor_import::PROVIDER.into();
    save(&core, &job, None).unwrap();
    core.import_job_running.store(true, Ordering::SeqCst);
    run(&core, job, &[home.clone()], None);
    let job = status(&core).unwrap().unwrap();
    println!(
        "MEASURE job_secs={:.1} status={} total={} imported={} left_out={} not_carried={}",
        started.elapsed().as_secs_f64(), job.status, job.total, job.imported, job.left_out, job.not_carried.len()
    );
    assert_eq!((job.imported, job.left_out), (1800, 200));
}

/// The Orca job against an Orca data folder (`$ORCA_USER_DATA_PATH`, else
/// this Mac's) and the agents' folders under `$HOME`, into a scratch
/// database. Writes each Orca chat's Bridge chat for the fidelity eval.
#[test]
#[ignore = "reads a real Orca data folder; run by hand for scripts/e2e/import-orca-fidelity.mjs"]
fn dump_the_orca_import_for_the_eval() {
    let out = std::env::var("ORCA_EVAL_OUT").expect("set ORCA_EVAL_OUT");
    let host = crate::provider_accounts::HostHomes::from_env();
    let paths = crate::orca_import::Paths::from_env(&host.home);
    let (store, _) = crate::orca_import::read(&paths).unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let core = core(scratch.path());
    let mut job = new_job();
    job.tool = crate::orca_import::PROVIDER.into();
    save(&core, &job, None).unwrap();
    core.import_job_running.store(true, Ordering::SeqCst);
    run(&core, job, &[paths.base.clone()], None);
    let job = status(&core).unwrap().unwrap();
    println!("EVAL job status={} imported={} failed={} workspaces={} not_carried={:?}", job.status, job.imported, job.failed, job.workspaces, job.not_carried);
    let db = core.db.lock().unwrap();
    let mut chats = Vec::new();
    for chat in crate::orca_import::chats(&store) {
        let by_native = chat.native.as_ref().and_then(|native| {
            db.query_row("SELECT session_id FROM import_native_sessions WHERE native_key=?1", params![native.key()], |row| row.get::<_, String>(0)).optional().unwrap()
        });
        let session = by_native.or_else(|| {
            db.query_row("SELECT id FROM sessions WHERE kind='imported' AND title=?1", params![chat.title.clone().unwrap_or_default()], |row| row.get::<_, String>(0)).optional().unwrap()
        });
        let Some(session) = session else {
            chats.push(json!({ "orcaId": chat.id, "missing": true }));
            continue;
        };
        let (title, parent): (Option<String>, Option<String>) = db
            .query_row("SELECT title, (SELECT p.title FROM sessions p WHERE p.id=s.parent_session_id) FROM sessions s WHERE id=?1", params![session], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap();
        let mut entries = db.prepare("SELECT kind, payload FROM session_entries WHERE session_id=?1 ORDER BY sequence").unwrap();
        let mut source = String::new();
        let texts: Vec<serde_json::Value> = entries
            .query_map([&session], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .filter_map(|(kind, payload)| {
                let payload: serde_json::Value = serde_json::from_str(&payload).ok()?;
                if source.is_empty() {
                    source = payload["importSource"].as_str().unwrap_or_default().to_owned();
                }
                let role = match kind.as_str() {
                    "user.message" => "user",
                    "assistant.message" => "assistant",
                    _ => return None,
                };
                Some(json!({ "role": role, "text": payload["text"].as_str().unwrap_or_default() }))
            })
            .collect();
        chats.push(json!({ "orcaId": chat.id, "title": title, "parent": parent, "source": source, "nativeKey": chat.native.as_ref().map(|native| native.key()), "entries": texts }));
    }
    fs::write(&out, serde_json::to_string_pretty(&chats).unwrap()).unwrap();
    println!("EVAL wrote {} chats to {out}", chats.len());
}

/// The Superset job against a Superset app home (`$SUPERSET_HOME_DIR`, else
/// this Mac's) and the agents' folders under `$HOME`, into a scratch
/// database. Writes each chat's and terminal session's Bridge chat for the
/// fidelity eval.
#[test]
#[ignore = "reads a real Superset app home; run by hand for scripts/e2e/import-superset-fidelity.mjs"]
fn dump_the_superset_import_for_the_eval() {
    let out = std::env::var("SUPERSET_EVAL_OUT").expect("set SUPERSET_EVAL_OUT");
    let host = crate::provider_accounts::HostHomes::from_env();
    let paths = crate::superset_import::Paths::from_env(&host.home);
    let store = crate::superset_import::read(&paths).unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let core = core(scratch.path());
    let mut job = new_job();
    job.tool = crate::superset_import::PROVIDER.into();
    save(&core, &job, None).unwrap();
    core.import_job_running.store(true, Ordering::SeqCst);
    run(&core, job, &paths.homes, None);
    let job = status(&core).unwrap().unwrap();
    println!("EVAL job status={} imported={} failed={} workspaces={} not_carried={:?}", job.status, job.imported, job.failed, job.workspaces, job.not_carried);
    let db = core.db.lock().unwrap();
    let units: Vec<(Option<String>, Option<String>)> = crate::superset_import::chats(&store)
        .map(|(_, chat)| (Some(chat.id.clone()), chat.native.as_ref().map(|native| native.key())))
        .chain(crate::superset_import::bindings(&store).map(|(_, binding)| (None, binding.native.as_ref().map(|native| native.key()))))
        .collect();
    let mut chats = Vec::new();
    for (superset_id, native_key) in units {
        let by_native = native_key.as_ref().and_then(|key| {
            db.query_row("SELECT session_id FROM import_native_sessions WHERE native_key=?1", params![key], |row| row.get::<_, String>(0)).optional().unwrap()
        });
        let by_source = superset_id.as_ref().and_then(|id| {
            db.query_row(
                "SELECT s.id FROM sessions s JOIN imported_record_provenance p ON p.bridge_id = s.id WHERE s.kind='imported' AND p.canonical_source_ref LIKE ?1 LIMIT 1",
                params![format!("%#{id}")],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .unwrap_or(None)
        });
        let Some(session) = by_native.or(by_source) else {
            chats.push(json!({ "supersetId": superset_id, "nativeKey": native_key, "missing": true }));
            continue;
        };
        let title: Option<String> = db.query_row("SELECT title FROM sessions WHERE id=?1", params![session], |row| row.get(0)).unwrap();
        let mut entries = db.prepare("SELECT kind, payload FROM session_entries WHERE session_id=?1 ORDER BY sequence").unwrap();
        let mut source = String::new();
        let texts: Vec<serde_json::Value> = entries
            .query_map([&session], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .filter_map(|(kind, payload)| {
                let payload: serde_json::Value = serde_json::from_str(&payload).ok()?;
                if source.is_empty() {
                    source = payload["importSource"].as_str().unwrap_or_default().to_owned();
                }
                let role = match kind.as_str() {
                    "user.message" => "user",
                    "assistant.message" => "assistant",
                    _ => return None,
                };
                Some(json!({ "role": role, "text": payload["text"].as_str().unwrap_or_default() }))
            })
            .collect();
        chats.push(json!({ "supersetId": superset_id, "nativeKey": native_key, "title": title, "source": source, "entries": texts }));
    }
    fs::write(&out, serde_json::to_string_pretty(&chats).unwrap()).unwrap();
    println!("EVAL wrote {} chats to {out}", chats.len());
}

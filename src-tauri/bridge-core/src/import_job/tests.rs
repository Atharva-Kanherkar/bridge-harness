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

mod codex {
    use super::*;

    fn line(kind: &str, payload: serde_json::Value) -> String {
        json!({ "timestamp": "2026-11-01T10:00:00Z", "ordinal": 0, "type": kind, "payload": payload }).to_string()
    }
    fn meta(id: &str, cwd: &str, parent: Option<&str>) -> String {
        let source = match parent {
            Some(parent) => json!({ "subagent": { "thread_spawn": { "parent_thread_id": parent } } }),
            None => json!("exec"),
        };
        line("session_meta", json!({ "id": id, "cwd": cwd, "timestamp": "2026-11-01T10:00:00Z", "originator": "codex_exec", "source": source }))
    }
    fn item(item: serde_json::Value) -> String {
        line("event_msg", json!({ "type": "item_completed", "item": item }))
    }
    fn said(id: &str, text: &str) -> String {
        item(json!({ "type": "UserMessage", "id": id, "content": [{ "type": "text", "text": text, "text_elements": [] }] }))
    }
    fn replied(id: &str, text: &str) -> String {
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

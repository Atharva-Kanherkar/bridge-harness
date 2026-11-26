use super::*;
use crate::api;
use bridge_protocol::messages::InputDisposition;
use std::io::{BufReader, Read};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

mod composer;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn lock<T>(mutex: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>, BridgeError> {
    mutex
        .lock()
        .map_err(|error| BridgeError::Invalid(error.to_string()))
}

const WORKSPACE: &str = "latency-workspace";
const FIXTURE_VERSION: u32 = 1;
const WARM_UPS: usize = 5;
const SAMPLES: usize = 60;
const FIRST_INPUT: &str = "Find the parser entry point.";
const NEXT_INPUT: &str = "Explain how it handles errors.";
const BACKGROUND_KINDS: [&str; 7] = [
    "connector",
    "suggestion",
    "briefing",
    "extraction",
    "outcome_evaluation",
    "consolidation",
    "chat_search",
];

#[derive(Clone, Copy)]
struct Size {
    chats: i64,
    background: i64,
    chat_entries: i64,
}

const SMALL: Size = Size {
    chats: 3,
    background: 7,
    chat_entries: 4,
};
const FULL: Size = Size {
    chats: 1_500,
    background: 50_000,
    chat_entries: 100,
};

struct Fixture {
    core: Arc<BridgeCore>,
    repo: PathBuf,
    size: Size,
    script: Shared,
    reader: Arc<Mutex<Option<ReaderControl>>>,
    _scratch: tempfile::TempDir,
}

impl Fixture {
    fn new(size: Size) -> TestResult<Self> {
        let scratch = tempfile::tempdir()?;
        let repo = repository(scratch.path())?;
        let mut core = BridgeCore::for_tests(scratch.path());
        let db = store::open(&core.database_path)?;
        seed(&db, &repo, size)?;
        core.db = Mutex::new(db);

        let script = Shared::default();
        let reader = Arc::new(Mutex::new(None));
        let adapter = BenchmarkAdapter {
            live: Live {
                id: "codex",
                script: script.clone(),
            },
            reader: reader.clone(),
        };
        let mut registry = AdapterRegistry::empty();
        registry.register(Box::new(adapter))?;
        core.adapter_registry = Arc::new(registry);
        Ok(Self {
            core: Arc::new(core),
            repo,
            size,
            script,
            reader,
            _scratch: scratch,
        })
    }

    fn check_counts(&self) -> TestResult {
        let db = lock(&self.core.db)?;
        let counts: (i64, i64, i64) = db.query_row(
            "SELECT (SELECT COUNT(*) FROM sessions WHERE kind='direct'),
                    (SELECT COUNT(*) FROM sessions WHERE kind<>'direct'),
                    (SELECT COUNT(*) FROM session_entries)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let Size {
            chats,
            background,
            chat_entries,
        } = self.size;
        assert_eq!(
            counts,
            (chats, background, chats * chat_entries + background * 3)
        );
        Ok(())
    }

    fn send_pair(&self, mut observe: impl FnMut(&'static str)) -> TestResult<[Duration; 3]> {
        let params = composer::create_params(&self.core, WORKSPACE)?;
        let (first, follow_up) = (FIRST_INPUT.to_owned(), NEXT_INPUT.to_owned());

        let started = Instant::now();
        let session = composer::create(&self.core, &params)?;
        observe("create");
        let input = api::prepare_turn(&self.core, session.clone(), first)?;
        observe("prepare");
        composer::start(&self.core, session.clone())?;
        observe("start");
        let result = api::submit_input_with_preference(
            &self.core,
            session.clone(),
            input.text,
            vec![],
            None,
        )?;
        let new_chat = started.elapsed();
        observe("submit");
        assert_eq!(result.disposition, InputDisposition::StartedNewTurn);
        let reader = lock(&self.reader)?
            .take()
            .ok_or("cold launch has no reader")?;

        let (thread, turn) = {
            let runtimes = lock(&self.core.adapters)?;
            let runtime = runtimes.get(&session).ok_or("cold launch has no runtime")?;
            (
                runtime.provider_session_id().to_owned(),
                runtime.current_turn(),
            )
        };
        self.frame(
            &session,
            &turn,
            &thread,
            "turn/started",
            json!({
                "turn": off_trace::codex_turn("first", "inProgress"),
            }),
        );
        let forest_entry = self.persist_command(&session, &turn, &thread)?;
        self.complete(&session, &turn, &thread, "first");
        self.check_turn(&session, FIRST_INPUT)?;
        observe("provider frames");

        let launches = lock(&self.script)?.asked.len();
        let started = Instant::now();
        let input = api::prepare_turn(&self.core, session.clone(), follow_up)?;
        observe("warm prepare");
        let result = api::submit_input_with_preference(
            &self.core,
            session.clone(),
            input.text,
            vec![],
            None,
        )?;
        let warm_send = started.elapsed();
        observe("warm submit");
        assert_eq!(result.disposition, InputDisposition::StartedNewTurn);
        {
            let mut script = lock(&self.script)?;
            assert_eq!(
                script.asked.len(),
                launches,
                "warm send relaunched the adapter"
            );
            let sent = std::mem::take(&mut script.sent);
            let [(first_launch, first), (warm_launch, warm)] = sent.as_slice() else {
                return Err("expected exactly two messages delivered to the adapter".into());
            };
            assert_eq!(first_launch, warm_launch);
            assert_eq!(first, FIRST_INPUT);
            assert_eq!(warm, NEXT_INPUT);
        }

        self.frame(
            &session,
            &turn,
            &thread,
            "turn/started",
            json!({
                "turn": off_trace::codex_turn("follow-up", "inProgress"),
            }),
        );
        self.complete(&session, &turn, &thread, "follow-up");
        self.check_turn(&session, NEXT_INPUT)?;
        api::archive_chat(&self.core, &session)?;
        reader.close()?;
        Ok([new_chat, warm_send, forest_entry])
    }

    fn persist_command(
        &self,
        session: &str,
        turn: &Arc<Mutex<Option<String>>>,
        thread: &str,
    ) -> TestResult<Duration> {
        let before = self.head(session)?.0;
        let started_at: String = lock(&self.core.db)?.query_row(
            "SELECT started_at FROM sessions WHERE id=?1",
            [session],
            |row| row.get(0),
        )?;
        let value = off_trace::codex(
            "item/started",
            json!({
                "threadId": thread,
                "turnId": "first",
                "item": { "type": "commandExecution", "id": "fixture-command", "command": "ls src", "status": "inProgress" },
            }),
        );
        // The benchmark represents a warm streaming entry. The handler still
        // runs its normal refresh path, and its time includes the append.
        let stamp = store::repository_state_for_path(&self.repo);
        assert_eq!(stamp["status"], "dirty");

        let started = Instant::now();
        handle_agent_value_timed(
            &self.core,
            session,
            turn,
            &value,
            None,
            Some((&started_at, thread)),
        );
        let elapsed = started.elapsed();

        assert_eq!(self.head(session)?, (before + 1, "command.started".into()));
        Ok(elapsed)
    }

    fn head(&self, session: &str) -> TestResult<(i64, String)> {
        Ok(lock(&self.core.db)?.query_row(
            "SELECT e.sequence,e.kind FROM session_heads h
             JOIN session_entries e ON e.id=h.active_entry_id WHERE h.session_id=?1",
            [session],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?)
    }

    fn frame(
        &self,
        session: &str,
        turn: &Arc<Mutex<Option<String>>>,
        thread: &str,
        method: &str,
        mut params: Value,
    ) {
        params["threadId"] = json!(thread);
        handle_agent_value(&self.core, session, turn, &off_trace::codex(method, params));
    }

    fn complete(&self, session: &str, turn: &Arc<Mutex<Option<String>>>, thread: &str, id: &str) {
        self.frame(session, turn, thread, "item/completed", json!({
            "turnId": id,
            "item": { "type": "agentMessage", "id": format!("{id}-reply"), "text": "The parser is in src/parser.rs.", "phase": "final_answer" },
        }));
        self.frame(
            session,
            turn,
            thread,
            "turn/completed",
            json!({
                "turn": off_trace::codex_turn(id, "completed"),
            }),
        );
    }

    fn check_turn(&self, session: &str, message: &str) -> TestResult {
        let db = lock(&self.core.db)?;
        let persisted: i64 = db.query_row(
            "SELECT COUNT(*) FROM session_entries WHERE session_id=?1 AND kind='user.message'
             AND json_extract(payload,'$.text')=?2",
            params![session, message],
            |row| row.get(0),
        )?;
        assert_eq!(persisted, 1);
        let active: Option<String> = db.query_row(
            "SELECT active_turn_id FROM sessions WHERE id=?1",
            [session],
            |row| row.get(0),
        )?;
        assert_eq!(active, None, "fake completion left the turn active");
        Ok(())
    }

    fn app_state(&self) -> TestResult<Duration> {
        let started = Instant::now();
        let state = api::get_app_state(&self.core)?;
        let encoded = serde_json::to_vec(&state)?;
        let elapsed = started.elapsed();
        assert!(!encoded.is_empty());
        assert_eq!(state.sessions.len() as i64, self.size.chats);
        assert!(state.sessions.iter().any(|session| session.id == "chat-0"));
        assert!(state
            .sessions
            .iter()
            .all(|session| !session.id.starts_with("run-")));
        Ok(elapsed)
    }
}

// The existing Live fixture tracks the test process as an adapter, sends
// through AdapterRuntime, and uses the real Codex frame normalization. Replace
// its indefinitely parked reader with a channel we close after every sample.
struct BenchmarkAdapter {
    live: Live,
    reader: Arc<Mutex<Option<ReaderControl>>>,
}

impl HarnessAdapter for BenchmarkAdapter {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn descriptor(&self) -> AdapterDescriptor {
        self.live.descriptor()
    }

    fn start(&self, request: StartRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        let mut started = self.live.start(request)?;
        let (close, wait_for_close) = mpsc::channel();
        let (ack, exited) = mpsc::channel();
        let mut reader = lock(&self.reader)?;
        if reader.is_some() {
            return Err(BridgeError::Invalid(
                "the preceding benchmark reader is still open".into(),
            ));
        }
        started.reader = Box::new(BufReader::new(BenchmarkReader {
            wait_for_close,
            ack,
        }));
        *reader = Some(ReaderControl { close, exited });
        Ok(started)
    }

    fn resume(&self, _: ResumeRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        Err(BridgeError::Invalid(
            "a benchmark chat unexpectedly resumed".into(),
        ))
    }

    fn supports_native_resume(&self) -> bool {
        false
    }

    fn normalize(&self, value: &Value) -> Vec<agent::NormalizedEvent> {
        self.live.normalize(value)
    }
}

struct BenchmarkReader {
    wait_for_close: Receiver<()>,
    ack: Sender<()>,
}

impl Read for BenchmarkReader {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        let _closed = self.wait_for_close.recv();
        Ok(0)
    }
}

impl Drop for BenchmarkReader {
    fn drop(&mut self) {
        let _acknowledged = self.ack.send(());
    }
}

struct ReaderControl {
    close: Sender<()>,
    exited: Receiver<()>,
}

impl ReaderControl {
    fn close(self) -> TestResult {
        drop(self.close);
        self.exited.recv_timeout(Duration::from_secs(5))?;
        Ok(())
    }
}

fn repository(root: &Path) -> TestResult<PathBuf> {
    let repo = root.join("workspace");
    std::fs::create_dir_all(repo.join("src"))?;
    for index in 0..48 {
        std::fs::write(
            repo.join(format!("src/file-{index}.rs")),
            "// Fixture code\n",
        )?;
    }
    git(&repo, &["init", "-q", "-b", "main"])?;
    git(&repo, &["add", "src"])?;
    git(
        &repo,
        &["commit", "-qm", "Seed latency fixture", "--no-gpg-sign"],
    )?;
    for index in 0..192 {
        std::fs::write(
            repo.join(format!("src/untracked-{index}")),
            "Fixture output\n",
        )?;
    }
    Ok(repo)
}

fn git(repo: &Path, args: &[&str]) -> TestResult {
    let output = crate::git::git_command(repo)
        .args([
            "-c",
            "user.name=Bridge benchmark",
            "-c",
            "user.email=benchmark@example.invalid",
        ])
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(args)
        .output()?;
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

fn seed(db: &rusqlite::Connection, repo: &Path, size: Size) -> TestResult {
    let path = repo.to_string_lossy();
    let tx = db.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO projects (id,name,path,created_at) VALUES ('latency-project','Parser',?1,'2026-01-01T00:00:00Z')",
        [&path],
    )?;
    tx.execute(
        "INSERT INTO workspaces (id,project_id,city,title,branch,path,status,created_at)
         VALUES (?1,'latency-project','Workspace','Parser','main',?2,'ready','2026-01-01T00:00:00Z')",
        params![WORKSPACE, path],
    )?;
    {
        let mut session = tx.prepare(
            "INSERT INTO sessions (id,workspace_id,harness,label,status,kind,model,cwd,depth,started_at,ended_at)
             VALUES (?1,?2,'codex',?3,?4,?5,'standard',?6,0,'2026-01-01T00:00:00Z','2026-01-01T00:02:00Z')",
        )?;
        let mut entry = tx.prepare(
            "INSERT INTO session_entries (id,session_id,parent_entry_id,sequence,semantic_schema_version,
                    kind,payload,context_visibility,token_estimate,created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'2026-01-01T00:00:00Z')",
        )?;
        let mut head = tx.prepare(
            "INSERT INTO session_heads (session_id,active_entry_id,updated_at)
             VALUES (?1,?2,'2026-01-01T00:02:00Z')",
        )?;
        for index in 0..size.chats + size.background {
            let (id, kind, label, status, entries) = if index < size.chats {
                (
                    format!("chat-{index}"),
                    "direct",
                    "Parser investigation",
                    "stopped",
                    size.chat_entries,
                )
            } else {
                let background = index - size.chats;
                let kind = BACKGROUND_KINDS[background as usize % BACKGROUND_KINDS.len()];
                (
                    format!("run-{background}"),
                    kind,
                    "Background run",
                    "completed",
                    3,
                )
            };
            session.execute(params![id, WORKSPACE, label, status, kind, path])?;
            let mut parent: Option<String> = None;
            for seq in 1..=entries {
                let (kind, payload) = seed_entry(seq, entries);
                let entry_id = format!("{id}-{seq}");
                let visible = if matches!(kind, "user.message" | "assistant.message") {
                    "eligible"
                } else {
                    "hidden"
                };
                entry.execute(params![
                    entry_id,
                    id,
                    parent,
                    seq,
                    crate::model::SEMANTIC_EVENT_SCHEMA_VERSION,
                    kind,
                    payload,
                    visible,
                    (visible == "eligible").then_some(22),
                ])?;
                parent = Some(entry_id);
            }
            head.execute(params![id, parent])?;
        }
    }
    tx.commit()?;
    Ok(())
}

fn seed_entry(sequence: i64, entries: i64) -> (&'static str, &'static str) {
    match (sequence % 4, sequence == entries) {
        (_, true) => (
            "turn.completed",
            r#"{"status":"completed","data":{"turnId":"fixture"}}"#,
        ),
        (1, _) => (
            "user.message",
            r#"{"role":"user","text":"Follow the parser from the caller and check how malformed input is handled."}"#,
        ),
        (2, _) => (
            "turn.started",
            r#"{"status":"inProgress","data":{"turnId":"fixture"}}"#,
        ),
        _ => (
            "assistant.message",
            r#"{"role":"assistant","text":"The parser returns a typed error from the input boundary and preserves its original location."}"#,
        ),
    }
}

#[test]
#[ignore = "full generated latency benchmark, run with --release --ignored --nocapture"]
fn measure() -> TestResult {
    let fixture = Fixture::new(FULL)?;
    fixture.check_counts()?;
    let mut samples: [Vec<Duration>; 4] = std::array::from_fn(|_| Vec::with_capacity(SAMPLES));
    for round in 0..WARM_UPS + SAMPLES {
        let [new_chat, warm_send, forest_entry] = fixture.send_pair(|_| {})?;
        let app_state = fixture.app_state()?;
        if round >= WARM_UPS {
            for (series, elapsed) in
                samples
                    .iter_mut()
                    .zip([new_chat, warm_send, forest_entry, app_state])
            {
                series.push(elapsed);
            }
        }
    }
    let names = ["new_chat", "warm_send", "forest_entry", "app_state"];
    let metrics: serde_json::Map<String, Value> = names
        .into_iter()
        .zip(samples)
        .map(|(name, mut series)| {
            series.sort_unstable();
            let n = series.len();
            let ms =
                |percent: usize| series[(n * percent).div_ceil(100) - 1].as_secs_f64() * 1000.0;
            (
                name.to_owned(),
                json!({"samples": n, "p50Ms": ms(50), "p95Ms": ms(95)}),
            )
        })
        .collect();
    println!(
        "\nBRIDGE_LATENCY {}",
        json!({"fixtureVersion": FIXTURE_VERSION, "metrics": metrics})
    );
    Ok(())
}

#[test]
fn fixture_has_visible_chats_hidden_runs_and_linked_history() -> TestResult {
    let fixture = Fixture::new(SMALL)?;
    fixture.check_counts()?;
    let db = lock(&fixture.core.db)?;
    let counts: (i64, i64, i64) = db.query_row(
        "SELECT (SELECT COUNT(*) FROM sessions WHERE kind='direct'),
                    (SELECT COUNT(DISTINCT kind) FROM sessions WHERE kind<>'direct'),
                    (SELECT COUNT(*) FROM session_entries)",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(counts, (3, 7, 33));
    let head: (String, i64, i64) = db.query_row(
        "SELECT e.kind,e.sequence,p.sequence FROM session_heads h
             JOIN session_entries e ON e.id=h.active_entry_id
             JOIN session_entries p ON p.id=e.parent_entry_id
             WHERE h.session_id='chat-0' AND p.session_id=h.session_id",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(head, ("turn.completed".into(), 4, 3));
    let cwd = store::repository_path_for_session(&db, "chat-0")?;
    assert_eq!(cwd.as_deref(), Some(fixture.repo.as_path()));
    assert!(fixture.repo.join(".git").is_dir());
    assert_eq!(
        store::repository_state_for_path(&fixture.repo)["status"],
        "dirty"
    );
    Ok(())
}

#[test]
fn latency_workload_delivers_both_turns_and_persists_its_frame() -> TestResult {
    check_small_workload(|_| {})
}

pub(super) fn check_small_workload(observe: impl FnMut(&'static str)) -> TestResult {
    let fixture = Fixture::new(SMALL)?;
    fixture.send_pair(observe)?;
    fixture.app_state()?;
    assert_eq!(api::get_state(&fixture.core)?.sessions.len(), 10);
    Ok(())
}

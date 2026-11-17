//! Warm connection runtimes: one hidden, turnless process per personal-app
//! connection, used when the chat asking is not itself that connection.
//!
//! A runtime is keyed by `{harness, account, server}`, so a call can never
//! reach another account's sign-in. It carries no Bridge tools and is never
//! sent a model turn:
//! - A Claude runtime is a briefing whose only authority is app control on
//!   its one server ([`BriefingRuntimePolicy::compile_app_runtime`]).
//! - A Codex runtime is `codex app-server` with an ephemeral thread and no
//!   Bridge MCP server.
//!
//! Concurrent first callers share one launch, a failed launch is not kept, a
//! runtime whose process ends is dropped, and one idle for
//! [`IDLE_LIMIT`] is stopped. Bridge exiting closes each runtime's stdin,
//! which ends it.

use super::control;
use crate::adapters::{AdapterRuntime, ShutdownReason, StartRequest};
use crate::briefing_policy::BriefingRuntimePolicy;
use crate::{runtime::BridgeCore, BridgeError};
use serde_json::Value;
use std::collections::HashMap;
use std::io::BufRead;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const IDLE_LIMIT: Duration = Duration::from_secs(10 * 60);
/// At most this many warm runtimes at once. A new one evicts the least
/// recently used idle runtime; when every runtime is busy it is refused.
pub const MAX_RUNTIMES: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key {
    pub harness: String,
    pub account: Option<String>,
    /// The project folder the connection resolves in; `None` is user-wide.
    pub cwd: Option<String>,
    pub server: String,
}

struct Warm {
    owner: String,
    runtime: Box<dyn AdapterRuntime>,
    last_used: Instant,
}

impl Drop for Warm {
    fn drop(&mut self) {
        self.runtime.stop(ShutdownReason::Completed);
    }
}

type Slot = Arc<Mutex<Option<Warm>>>;

/// A started runtime and the stream its replies arrive on.
pub struct Launched {
    pub runtime: Box<dyn AdapterRuntime>,
    pub reader: Box<dyn BufRead + Send>,
}

pub type Launcher = dyn Fn(&Key) -> Result<Launched, BridgeError> + Send + Sync;

struct Pool {
    slots: Mutex<HashMap<Key, Slot>>,
}

fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| {
        std::thread::Builder::new()
            .name("app-runtime-reaper".into())
            .spawn(|| loop {
                std::thread::sleep(Duration::from_secs(30));
                reap(IDLE_LIMIT);
            })
            .ok();
        Pool { slots: Mutex::new(HashMap::new()) }
    })
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn slot(key: &Key) -> Slot {
    lock(&pool().slots).entry(key.clone()).or_default().clone()
}

/// Stop every runtime idle for longer than `limit`.
pub fn reap(limit: Duration) {
    reap_matching(limit, |_| true);
}

fn reap_matching(limit: Duration, matches: impl Fn(&Key) -> bool) {
    let slots: Vec<Slot> = lock(&pool().slots).iter().filter(|(key, _)| matches(key)).map(|(_, slot)| slot.clone()).collect();
    for slot in slots {
        // A slot in use (launching or sending) is not idle.
        let Ok(mut warm) = slot.try_lock() else { continue };
        if warm.as_ref().is_some_and(|warm| warm.last_used.elapsed() > limit) {
            warm.take();
        }
    }
}

/// Stop the runtime for `key`, if there is one: a discovery that found no
/// such server leaves nothing behind.
pub fn retire(key: &Key) {
    let slot = lock(&pool().slots).get(key).cloned();
    if let Some(slot) = slot {
        lock(&slot).take();
    }
}

/// Make room for one more runtime, before any process starts. Forgets empty
/// slots, then evicts the least recently used idle runtime when full.
fn admit(key: &Key) -> Result<(), BridgeError> {
    let mut slots = lock(&pool().slots);
    slots.retain(|candidate, slot| candidate == key || slot.try_lock().map_or(true, |warm| warm.is_some()));
    let mut live = 0;
    let mut oldest: Option<(Instant, Slot)> = None;
    for (candidate, slot) in slots.iter() {
        if candidate == key {
            continue;
        }
        match slot.try_lock() {
            Ok(warm) => {
                if let Some(warm) = warm.as_ref() {
                    live += 1;
                    if oldest.as_ref().is_none_or(|(used, _)| warm.last_used < *used) {
                        oldest = Some((warm.last_used, slot.clone()));
                    }
                }
            }
            // In use (launching or sending): live, and not evictable.
            Err(_) => live += 1,
        }
    }
    drop(slots);
    if live < MAX_RUNTIMES {
        return Ok(());
    }
    match oldest {
        Some((_, slot)) => {
            if let Ok(mut warm) = slot.try_lock() {
                warm.take();
            }
            Ok(())
        }
        None => Err(BridgeError::Invalid("Too many app connections are busy; try again in a moment".into())),
    }
}

/// One app-control request on the warm runtime for `key`, launching it with
/// `launch` first when there is none.
pub fn request_with(key: &Key, request: &Value, launch: &Launcher) -> Result<Value, BridgeError> {
    let slot = slot(key);
    let owner = {
        // Held across a launch, so concurrent first callers share it.
        let mut warm = lock(&slot);
        if warm.is_none() {
            admit(key)?;
            *warm = Some(start(key, launch, &slot)?);
        }
        let Some(warm) = warm.as_mut() else {
            return Err(BridgeError::Invalid("The connection could not start".into()));
        };
        warm.last_used = Instant::now();
        warm.owner.clone()
    };
    control::request_with(&owner, |id| {
        let warm = lock(&slot);
        let warm = warm
            .as_ref()
            .filter(|warm| warm.owner == owner)
            .ok_or_else(|| BridgeError::Invalid("The connection stopped; try again".into()))?;
        warm.runtime.app_control(id, request)
    })
}

fn start(key: &Key, launch: &Launcher, slot: &Slot) -> Result<Warm, BridgeError> {
    // One retry: two agents starting against a fresh home can race to create
    // its state database, and the loser exits during initialization.
    let Launched { runtime, mut reader } = match launch(key) {
        Ok(launched) => launched,
        Err(_) => {
            std::thread::sleep(Duration::from_millis(400));
            launch(key)?
        }
    };
    let owner = format!("app-runtime:{}", uuid::Uuid::new_v4());
    let reader_owner = owner.clone();
    let reader_slot = Arc::downgrade(slot);
    std::thread::Builder::new()
        .name("app-runtime-reader".into())
        .spawn(move || {
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if let Ok(value) = serde_json::from_str::<Value>(line.trim()) {
                            let value = super::codex::translate(&value).unwrap_or(value);
                            control::deliver(&reader_owner, &value);
                        }
                    }
                }
            }
            // The process ended: forget it, unless a newer one took the slot.
            if let Some(slot) = reader_slot.upgrade() {
                let mut warm = lock(&slot);
                if warm.as_ref().is_some_and(|warm| warm.owner == reader_owner) {
                    warm.take();
                }
            }
        })
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    Ok(Warm { owner, runtime, last_used: Instant::now() })
}

/// The production launcher: the connection's harness, its account, no tools.
pub fn launcher(core: &Arc<BridgeCore>) -> impl Fn(&Key) -> Result<Launched, BridgeError> + Send + Sync {
    let core = core.clone();
    move |key: &Key| launch(&core, key)
}

fn launch(core: &Arc<BridgeCore>, key: &Key) -> Result<Launched, BridgeError> {
    let account = {
        let db = core.try_db()?;
        key.account
            .as_deref()
            .map(|id| crate::agent_config::account(&db, &key.harness, id))
            .transpose()?
    };
    let account = account
        .map(|account| {
            crate::provider_accounts::prepare_launch(
                &key.harness,
                &account,
                &crate::provider_accounts::HostHomes::from_env(),
            )
        })
        .transpose()?;
    // A project connection resolves its servers in that project, exactly as
    // the chat that found it did; a missing folder is refused rather than
    // resolving to the user-wide server of the same name.
    let cwd = match &key.cwd {
        Some(project) if std::path::Path::new(project).is_dir() => project.clone(),
        Some(_) => {
            return Err(BridgeError::Invalid(format!(
                "The project folder this app's {} connection lives in is gone",
                key.server
            )))
        }
        None => {
            let scratch = core.chat_scratch_dir(&format!("app-runtime-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&scratch)?;
            scratch.to_string_lossy().to_string()
        }
    };
    let policy = match key.harness.as_str() {
        "claude" => Some(
            BriefingRuntimePolicy::compile_app_runtime(
                key.server.clone(),
                crate::connector_runs::run_limits(crate::connector_runs::RunKind::Ingress),
            )
            .map_err(|error| BridgeError::Invalid(error.reason()))?,
        ),
        "codex" => None,
        other => {
            return Err(BridgeError::Invalid(format!(
                "{} cannot run personal apps directly",
                super::router::agent_label(other)
            )))
        }
    };
    let start = || {
        core.adapter_registry.start(
            &key.harness,
            StartRequest {
                session_id: None,
                cwd: &cwd,
                model: None,
                effort: None,
                instructions: None,
                // Codex: an explicit mode, so the launch never attaches Bridge's
                // own tools (a launch without one keeps its old meaning and
                // does). It is never sent a turn, so the mode grants nothing.
                // Claude: the briefing alone carries the authority.
                write_mode: policy.is_none().then_some(crate::delegation::WriteMode::Shared),
                read_only_sandbox: None,
                briefing: policy.as_ref(),
                interactive: false,
                account: account.as_ref(),
                on_progress: None,
            },
        )
    };
    let started = start()?;
    Ok(Launched { runtime: started.runtime, reader: started.reader })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{BufReader, Read};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;

    /// A reader fed by the fake runtime: each app_control call answers itself.
    struct Pipe(mpsc::Receiver<Vec<u8>>, Vec<u8>);
    impl Read for Pipe {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.1.is_empty() {
                match self.0.recv() {
                    Ok(bytes) => self.1 = bytes,
                    Err(_) => return Ok(0),
                }
            }
            let n = buf.len().min(self.1.len());
            buf[..n].copy_from_slice(&self.1[..n]);
            self.1.drain(..n);
            Ok(n)
        }
    }

    struct Fake {
        out: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
        label: String,
        stopped: Arc<AtomicUsize>,
    }
    impl AdapterRuntime for Fake {
        fn process_id(&self) -> u32 { 0 }
        fn provider_session_id(&self) -> &str { "fake" }
        fn current_turn(&self) -> Arc<Mutex<Option<String>>> { Arc::new(Mutex::new(None)) }
        fn send_turn(&self, _: &str) -> Result<(), BridgeError> { panic!("a runtime never gets a turn") }
        fn interrupt(&self) -> Result<(), BridgeError> { Ok(()) }
        fn respond(&self, _: Value, _: &str) -> Result<(), BridgeError> { Ok(()) }
        fn stop(&mut self, _: ShutdownReason) {
            self.stopped.fetch_add(1, Ordering::SeqCst);
            self.out.lock().unwrap().take();
        }
        fn app_control(&self, id: &str, request: &Value) -> Result<(), BridgeError> {
            let frame = json!({"type":"app_control_result","id":id,"ok":true,"result":{"via":self.label,"server":request["server"]}});
            if let Some(out) = self.out.lock().unwrap().as_ref() {
                out.send(format!("{frame}\n").into_bytes()).unwrap();
            }
            Ok(())
        }
    }

    fn fake_launcher(launches: Arc<AtomicUsize>, stopped: Arc<AtomicUsize>) -> Box<Launcher> {
        Box::new(move |key: &Key| {
            launches.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(50));
            let (tx, rx) = mpsc::channel();
            Ok(Launched {
                runtime: Box::new(Fake {
                    out: Mutex::new(Some(tx)),
                    label: format!("{}:{}", key.harness, key.account.clone().unwrap_or_default()),
                    stopped: stopped.clone(),
                }),
                reader: Box::new(BufReader::new(Pipe(rx, Vec::new()))),
            })
        })
    }

    /// The pool is process-wide, so the tests that fill or reap it take turns.
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static TURN: Mutex<()> = Mutex::new(());
        TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn key(account: Option<&str>) -> Key {
        Key { harness: "codex".into(), account: account.map(str::to_owned), cwd: None, server: format!("srv-{}", uuid::Uuid::new_v4()) }
    }

    #[test]
    fn concurrent_callers_share_one_launch_and_accounts_stay_apart() {
        let _turn = serial();
        let launches = Arc::new(AtomicUsize::new(0));
        let stopped = Arc::new(AtomicUsize::new(0));
        let launch: Arc<Box<Launcher>> = Arc::new(fake_launcher(launches.clone(), stopped));
        let shared = key(None);
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let (key, launch) = (shared.clone(), launch.clone());
                std::thread::spawn(move || request_with(&key, &json!({"kind":"inventory","server":key.server}), launch.as_ref().as_ref()).unwrap())
            })
            .collect();
        for thread in threads {
            assert_eq!(thread.join().unwrap()["via"], "codex:");
        }
        assert_eq!(launches.load(Ordering::SeqCst), 1);
        let mut work = shared.clone();
        work.account = Some("work".into());
        let reply = request_with(&work, &json!({"kind":"inventory","server":work.server}), launch.as_ref().as_ref()).unwrap();
        assert_eq!(reply["via"], "codex:work", "another account is another runtime");
        assert_eq!(launches.load(Ordering::SeqCst), 2);
        let mut project = shared.clone();
        project.cwd = Some("/repo".into());
        request_with(&project, &json!({"kind":"inventory","server":project.server}), launch.as_ref().as_ref()).unwrap();
        assert_eq!(launches.load(Ordering::SeqCst), 3, "a project connection is its own runtime");
    }

    /// A real `codex app-server` against the fake MCP server, through the pool
    /// and the Codex adapter: inventory with annotations and a direct read, no
    /// model turn. Uses a temporary `CODEX_HOME`, never the person's own.
    #[test]
    #[ignore = "needs codex on PATH; run with --ignored"]
    fn live_codex_runtime_inventories_and_reads_without_a_turn() {
        let home = tempfile::tempdir().unwrap();
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testing/fixtures/fake-mcp-server.mjs");
        std::fs::write(
            home.path().join("config.toml"),
            format!("[mcp_servers.fake]\ncommand = \"node\"\nargs = [\"{}\"]\n", fixture.display()),
        )
        .unwrap();
        // SAFETY: an ignored test run on its own; nothing else reads the variable concurrently.
        unsafe { std::env::set_var("CODEX_HOME", home.path()) };
        let registry = Arc::new(crate::adapters::AdapterRegistry::built_in().unwrap());
        let ready = Instant::now();
        while !registry.descriptors().iter().any(|d| d.id == "codex" && d.available) {
            assert!(ready.elapsed() < Duration::from_secs(30), "codex was not discovered");
            std::thread::sleep(Duration::from_millis(200));
        }
        let cwd = home.path().to_string_lossy().to_string();
        let launch_registry = registry.clone();
        let launcher = move |key: &Key| -> Result<Launched, BridgeError> {
            let started = launch_registry.start(&key.harness, StartRequest {
                session_id: None, cwd: &cwd, model: None, effort: None, instructions: None, write_mode: None,
                read_only_sandbox: None, briefing: None, interactive: false, account: None, on_progress: None,
            })?;
            Ok(Launched { runtime: started.runtime, reader: started.reader })
        };
        let key = Key { harness: "codex".into(), account: None, cwd: None, server: "fake".into() };
        let inventory = request_with(&key, &json!({"kind":"inventory","server":"fake"}), &launcher).unwrap();
        assert_eq!(inventory["status"], "connected", "{inventory}");
        assert_eq!(inventory["tools"], json!([
            {"name":"search_people","readOnly":true,"destructive":null},
            {"name":"send_note","readOnly":null,"destructive":true}
        ]));
        let read = request_with(&key, &json!({"kind":"read","server":"fake","tool":"search_people","arguments":{"query":"Ada"}}), &launcher).unwrap();
        assert_eq!(read["content"][0]["text"], "Found Ada Lovelace", "{read}");
        reap_matching(Duration::ZERO, |candidate| candidate == &key);
    }

    /// The review's reproduction: the user-wide `review` server is tenant A
    /// and a trusted project overrides `review` as tenant B. A warm runtime
    /// for the project connection must read B, and the user-wide one A.
    #[test]
    #[ignore = "needs codex on PATH; run with --ignored"]
    fn live_codex_project_connection_never_becomes_the_user_wide_one() {
        let home = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let project_path = project.path().canonicalize().unwrap();
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testing/fixtures/fake-mcp-server.mjs");
        let server = |label: &str| format!("[mcp_servers.review]\ncommand = \"node\"\nargs = [\"{}\"]\nenv = {{ FAKE_MCP_LABEL = \"{label}\" }}\n", fixture.display());
        std::fs::write(
            home.path().join("config.toml"),
            format!("{}\n[projects.\"{}\"]\ntrust_level = \"trusted\"\n", server("A"), project_path.display()),
        )
        .unwrap();
        std::fs::create_dir_all(project_path.join(".codex")).unwrap();
        std::fs::write(project_path.join(".codex/config.toml"), server("B")).unwrap();
        // SAFETY: an ignored test run on its own.
        unsafe { std::env::set_var("CODEX_HOME", home.path()) };
        let registry = Arc::new(crate::adapters::AdapterRegistry::built_in().unwrap());
        let ready = Instant::now();
        while !registry.descriptors().iter().any(|d| d.id == "codex" && d.available) {
            assert!(ready.elapsed() < Duration::from_secs(30), "codex was not discovered");
            std::thread::sleep(Duration::from_millis(200));
        }
        let scratch = home.path().to_string_lossy().to_string();
        let launcher = move |key: &Key| -> Result<Launched, BridgeError> {
            let cwd = key.cwd.clone().unwrap_or_else(|| scratch.clone());
            let started = registry.start(&key.harness, StartRequest {
                session_id: None, cwd: &cwd, model: None, effort: None, instructions: None, write_mode: None,
                read_only_sandbox: None, briefing: None, interactive: false, account: None, on_progress: None,
            })?;
            Ok(Launched { runtime: started.runtime, reader: started.reader })
        };
        let read = |key: &Key| request_with(key, &json!({"kind":"read","server":"review","tool":"search_people","arguments":{"query":"Ada"}}), &launcher).unwrap();
        let user_wide = Key { harness: "codex".into(), account: None, cwd: None, server: "review".into() };
        let in_project = Key { cwd: Some(project_path.to_string_lossy().into_owned()), ..user_wide.clone() };
        assert_eq!(read(&in_project)["content"][0]["text"], "[B] Found Ada Lovelace");
        assert_eq!(read(&user_wide)["content"][0]["text"], "[A] Found Ada Lovelace");
        reap_matching(Duration::ZERO, |candidate| candidate.server == "review");
    }

    /// A real Claude sidecar launched as an app runtime (a briefing that never
    /// gets a prompt) answers inventory and a read for its one server, and
    /// refuses another server. Uses a temporary `CLAUDE_CONFIG_DIR`.
    #[test]
    #[ignore = "needs claude and the sidecar; run with --ignored"]
    fn live_claude_app_runtime_serves_only_its_server_without_a_turn() {
        let home = tempfile::tempdir().unwrap();
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testing/fixtures/fake-mcp-server.mjs");
        std::fs::write(
            home.path().join(".claude.json"),
            json!({"mcpServers": {"fake": {"type": "stdio", "command": "node", "args": [fixture.to_string_lossy()]}}}).to_string(),
        )
        .unwrap();
        // SAFETY: an ignored test run on its own.
        unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", home.path()) };
        let registry = Arc::new(crate::adapters::AdapterRegistry::built_in().unwrap());
        let ready = Instant::now();
        while !registry.descriptors().iter().any(|d| d.id == "claude" && d.available) {
            assert!(ready.elapsed() < Duration::from_secs(60), "claude was not discovered");
            std::thread::sleep(Duration::from_millis(200));
        }
        let cwd = home.path().to_string_lossy().to_string();
        let launch_registry = registry.clone();
        let launcher = move |key: &Key| -> Result<Launched, BridgeError> {
            let policy = BriefingRuntimePolicy::compile_app_runtime(key.server.clone(), crate::connector_runs::run_limits(crate::connector_runs::RunKind::Ingress)).unwrap();
            let started = launch_registry.start(&key.harness, StartRequest {
                session_id: None, cwd: &cwd, model: None, effort: None, instructions: None, write_mode: None,
                read_only_sandbox: None, briefing: Some(&policy), interactive: false, account: None, on_progress: None,
            })?;
            Ok(Launched { runtime: started.runtime, reader: started.reader })
        };
        let key = Key { harness: "claude".into(), account: None, cwd: None, server: "fake".into() };
        let inventory = request_with(&key, &json!({"kind":"inventory","server":"fake"}), &launcher).unwrap();
        assert_eq!(inventory["status"], "connected", "{inventory}");
        let read = request_with(&key, &json!({"kind":"read","server":"fake","tool":"search_people","arguments":{"query":"Ada"}}), &launcher).unwrap();
        assert!(read.to_string().contains("Found Ada Lovelace"), "{read}");
        let write = request_with(&key, &json!({"kind":"read","server":"fake","tool":"send_note","arguments":{}}), &launcher);
        assert!(write.is_err(), "a write is never served");
        let other = request_with(&key, &json!({"kind":"inventory","server":"other"}), &launcher);
        assert!(other.is_err(), "another server is never served");
        reap_matching(Duration::ZERO, |candidate| candidate == &key);
    }

    #[test]
    fn a_project_connection_whose_folder_is_gone_is_refused_not_widened() {
        let dir = tempfile::tempdir().unwrap();
        let core = Arc::new(BridgeCore::for_tests(dir.path()));
        let key = Key { harness: "codex".into(), account: None, cwd: Some("/nonexistent/bridge-project".into()), server: "review".into() };
        let error = launch(&core, &key).err().expect("refused").to_string();
        assert!(error.contains("project folder"), "{error}");
    }

    #[test]
    fn the_pool_is_bounded_and_retired_runtimes_stop() {
        let _turn = serial();
        let launches = Arc::new(AtomicUsize::new(0));
        let stopped = Arc::new(AtomicUsize::new(0));
        let launch = fake_launcher(launches.clone(), stopped.clone());
        let mine: Vec<Key> = (0..MAX_RUNTIMES + 2).map(|_| key(None)).collect();
        for k in &mine {
            request_with(k, &json!({"kind":"inventory","server":k.server}), launch.as_ref()).unwrap();
        }
        let live = lock(&pool().slots).values().filter(|slot| slot.try_lock().map_or(true, |warm| warm.is_some())).count();
        assert!(live <= MAX_RUNTIMES, "{live} warm runtimes");
        assert!(stopped.load(Ordering::SeqCst) >= 2, "the least recently used were evicted");
        let before = stopped.load(Ordering::SeqCst);
        let last = mine.last().unwrap();
        retire(last);
        assert_eq!(stopped.load(Ordering::SeqCst), before + 1, "a retired runtime stops");
        retire(&key(None));
        assert_eq!(stopped.load(Ordering::SeqCst), before + 1, "retiring an unknown connection is a no-op");
        for k in &mine {
            retire(k);
        }
    }

    #[test]
    fn failed_launches_are_not_kept_and_idle_runtimes_stop() {
        let _turn = serial();
        let k = key(None);
        let failing: Box<Launcher> = Box::new(|_: &Key| Err(BridgeError::Invalid("no codex".into())));
        assert!(request_with(&k, &json!({"kind":"inventory","server":"x"}), failing.as_ref()).is_err());
        let launches = Arc::new(AtomicUsize::new(0));
        let stopped = Arc::new(AtomicUsize::new(0));
        let launch = fake_launcher(launches.clone(), stopped.clone());
        request_with(&k, &json!({"kind":"inventory","server":"x"}), launch.as_ref()).unwrap();
        assert_eq!(launches.load(Ordering::SeqCst), 1, "the failure was not cached");
        let mine = |candidate: &Key| candidate == &k;
        reap_matching(Duration::from_secs(3600), mine);
        assert_eq!(stopped.load(Ordering::SeqCst), 0, "a fresh runtime is kept");
        std::thread::sleep(Duration::from_millis(20));
        reap_matching(Duration::from_millis(1), mine);
        assert_eq!(stopped.load(Ordering::SeqCst), 1, "an idle runtime is stopped");
        request_with(&k, &json!({"kind":"inventory","server":"x"}), launch.as_ref()).unwrap();
        assert_eq!(launches.load(Ordering::SeqCst), 2, "the next call starts a new one");
    }
}

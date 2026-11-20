//! The world of the early compaction tests: warm Claude, Codex and OpenCode
//! chats on the trace harness, levers on the adapter calls, and turn frames
//! with a window reading of a known size.

use super::off_trace::{
    claude_assistant, claude_compaction, claude_context_usage, claude_frame, claude_init, claude_result,
    claude_status, claude_usage, codex, codex_compaction_end, codex_compaction_start, codex_item, codex_tokens,
    codex_turn, codex_turn_completed, codex_usage, CallLog, TraceHarness, TraceRuntime, CLAUDE_CHAT, CLAUDE_MODEL,
    CLAUDE_THREAD, CODEX_CHAT, CODEX_MODEL, CODEX_THREAD,
};
use super::*;
use crate::adapters::{AdapterRuntime, InterruptRequest, NativeCompaction, ShutdownReason, TurnContext};
use crate::early_compaction::{DueMark, Receipt};
use bridge_protocol::messages::{SaveEarlyCompactionSettingsParams, TurnImage};
use regex::Regex;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{MutexGuard, PoisonError, Weak};
use std::thread::{JoinHandle, ThreadId};

pub(super) type TestResult = Result<(), Box<dyn std::error::Error>>;
pub(super) type Outcome<T> = Result<T, Box<dyn std::error::Error>>;
type Slot<T> = Arc<Mutex<Option<T>>>;
type Hook = Box<dyn FnOnce() + Send>;

pub(super) const OPENCODE_CHAT: &str = "opencode-chat";
const OPENCODE_THREAD: &str = "opencode-thread-1";
/// The chats that the rule can fire on.
pub(super) const FIRING_CHATS: [&str; 2] = [CLAUDE_CHAT, CODEX_CHAT];
/// Above `THRESHOLD_TOKENS`, so a bound reading fires.
pub(super) const LARGE: u64 = 150_000;
/// The window that a compaction leaves, below the threshold.
pub(super) const SMALL: u64 = 35_752;
/// The figures of the Claude boundary in these frames.
pub(super) const PRE_TOKENS: i64 = 45_586;
pub(super) const POST_TOKENS: i64 = 13_654;
/// A sweep this long after a turn end finds the mark past the idle delay
/// and still warm.
pub(super) const WARM_SECONDS: i64 = 30;
/// A thread that is still blocked after this time waits for a lock.
const WAIT: std::time::Duration = std::time::Duration::from_millis(200);
const ANSWER: &str = "The file has 400 numbered lines.";
/// The text of a send while a compaction can run.
pub(super) const FOLLOW_UP: &str = "And list the line numbers.";
/// A prompt change fence with a body that does not parse. The frame path
/// records the control turn and creates no proposal.
const CONTROL_REPLY: &str = "```bridge-prompt-change\nnot json\n```";
const CHATS: [(&str, &str, &str); 3] = [
    (CLAUDE_CHAT, "claude", CLAUDE_THREAD),
    (CODEX_CHAT, "codex", CODEX_THREAD),
    (OPENCODE_CHAT, "opencode", OPENCODE_THREAD),
];

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Levers on the adapter calls, shared by each runtime of one world.
#[derive(Clone, Default)]
pub(super) struct Levers {
    compact_hook: Slot<Hook>,
    send_hook: Slot<Hook>,
    support_hook: Slot<Hook>,
    support_answers: Arc<Mutex<VecDeque<Option<NativeCompaction>>>>,
    error: Slot<String>,
    /// One value for each send that reached a runtime: whether a holder of
    /// the input lease kept the sweep out while it ran.
    pub(super) leased_sends: Arc<Mutex<Vec<bool>>>,
    /// One entry for each call to a harness to start or resume a process.
    pub(super) launches: Arc<Mutex<Vec<String>>>,
}

impl Levers {
    /// Run `hook` inside the next adapter call of a compaction, before the
    /// call writes anything.
    pub(super) fn on_compact(&self, hook: impl FnOnce() + Send + 'static) {
        *lock(&self.compact_hook) = Some(Box::new(hook));
    }

    /// Run `hook` inside the next adapter call of a send.
    pub(super) fn on_send(&self, hook: impl FnOnce() + Send + 'static) {
        *lock(&self.send_hook) = Some(Box::new(hook));
    }

    /// Run `hook` inside the next question to a runtime about its native
    /// compaction. The sweep asks it after its memory check and before its
    /// decision.
    pub(super) fn on_support(&self, hook: impl FnOnce() + Send + 'static) {
        *lock(&self.support_hook) = Some(Box::new(hook));
    }

    /// Give these answers to the next questions about native compaction, in
    /// order. `None` gives the answer of the trace runtime.
    pub(super) fn answer_support(&self, answers: impl IntoIterator<Item = Option<NativeCompaction>>) {
        lock(&self.support_answers).extend(answers);
    }

    /// Each later adapter call of a compaction returns this error.
    pub(super) fn fail_compact(&self, error: &str) {
        *lock(&self.error) = Some(error.to_owned());
    }
}

/// The trace runtime with the levers in front of it.
struct EarlyRuntime {
    inner: TraceRuntime,
    levers: Levers,
    core: Weak<BridgeCore>,
}

impl EarlyRuntime {
    fn note_send(&self) {
        let leased = self.core.upgrade().is_some_and(|core| core.input_activity.try_write().is_err());
        lock(&self.levers.leased_sends).push(leased);
        let hook = lock(&self.levers.send_hook).take();
        if let Some(hook) = hook {
            hook();
        }
    }
}

impl AdapterRuntime for EarlyRuntime {
    fn process_id(&self) -> u32 {
        self.inner.process_id()
    }
    fn provider_session_id(&self) -> &str {
        self.inner.provider_session_id()
    }
    fn current_turn(&self) -> Arc<Mutex<Option<String>>> {
        self.inner.current_turn()
    }
    fn send_turn(&self, text: &str) -> Result<(), BridgeError> {
        self.note_send();
        self.inner.send_turn(text)
    }
    fn send_turn_with_context(&self, text: &str, context: TurnContext<'_>) -> Result<(), BridgeError> {
        self.note_send();
        self.inner.send_turn_with_context(text, context)
    }
    fn supports_images(&self) -> bool {
        self.inner.supports_images()
    }
    fn send_turn_with_images(&self, text: &str, context: TurnContext<'_>, images: &[TurnImage]) -> Result<(), BridgeError> {
        self.note_send();
        self.inner.send_turn_with_images(text, context, images)
    }
    fn supports_active_turn_steering(&self) -> bool {
        self.inner.supports_active_turn_steering()
    }
    fn steer_requires_acknowledged_turn(&self) -> bool {
        self.inner.steer_requires_acknowledged_turn()
    }
    fn steer_turn(&self, text: &str, context: TurnContext<'_>, images: &[TurnImage]) -> Result<(), BridgeError> {
        self.note_send();
        self.inner.steer_turn(text, context, images)
    }
    fn interrupt(&self) -> Result<(), BridgeError> {
        self.inner.interrupt()
    }
    fn interrupt_request(&self) -> Option<InterruptRequest> {
        self.inner.interrupt_request()
    }
    fn respond(&self, request_id: Value, decision: &str) -> Result<(), BridgeError> {
        self.inner.respond(request_id, decision)
    }
    fn answer_question(&self, request_id: Value, answers: Value) -> Result<(), BridgeError> {
        self.inner.answer_question(request_id, answers)
    }
    fn reject_question(&self, request_id: Value) -> Result<(), BridgeError> {
        self.inner.reject_question(request_id)
    }
    fn read_usage(&self) -> Result<(), BridgeError> {
        self.inner.read_usage()
    }
    fn native_compaction(&self) -> NativeCompaction {
        let hook = lock(&self.levers.support_hook).take();
        if let Some(hook) = hook {
            hook();
        }
        let answer = lock(&self.levers.support_answers).pop_front().flatten();
        answer.unwrap_or_else(|| self.inner.native_compaction())
    }
    fn compact_native(&self, focus: Option<&str>) -> Result<(), BridgeError> {
        let hook = lock(&self.levers.compact_hook).take();
        if let Some(hook) = hook {
            hook();
        }
        if let Some(error) = lock(&self.levers.error).clone() {
            return Err(BridgeError::Invalid(error));
        }
        self.inner.compact_native(focus)
    }
    fn stop(&mut self, reason: ShutdownReason) {
        self.inner.stop(reason);
    }
}

/// The trace harness with a record of each start and each resume.
struct EarlyHarness {
    id: &'static str,
    inner: TraceHarness,
    launches: Arc<Mutex<Vec<String>>>,
}

impl HarnessAdapter for EarlyHarness {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn descriptor(&self) -> AdapterDescriptor {
        self.inner.descriptor()
    }
    fn start(&self, request: StartRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        lock(&self.launches).push(format!("start {}", self.id));
        self.inner.start(request)
    }
    fn resume(&self, request: ResumeRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        lock(&self.launches).push(format!("resume {}", self.id));
        self.inner.resume(request)
    }
    fn supports_native_resume(&self) -> bool {
        self.inner.supports_native_resume()
    }
    fn supports_native_compaction(&self) -> bool {
        self.inner.supports_native_compaction()
    }
    fn normalize(&self, value: &Value) -> Vec<agent::NormalizedEvent> {
        self.inner.normalize(value)
    }
    fn forget_session(&self, provider_session_id: &str) {
        self.inner.forget_session(provider_session_id);
    }
}

/// One row of `early_compaction_attempts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Attempt {
    pub(super) id: String,
    pub(super) outcome: Option<String>,
    pub(super) dispatched: bool,
    pub(super) pre: Option<i64>,
    pub(super) post: Option<i64>,
    pub(super) compaction_turn_id: Option<String>,
    pub(super) trigger_turn_id: Option<String>,
    pub(super) trigger_reading_id: i64,
    pub(super) trigger_used_tokens: i64,
    pub(super) next_reading_id: Option<i64>,
    pub(super) next_used_tokens: Option<i64>,
    pub(super) error: Option<String>,
    pub(super) settled_at: Option<String>,
    pub(super) reason: String,
    pub(super) launch_started_at: String,
}

/// One row of `context_readings`: the id, the turn id and the used tokens.
pub(super) type ReadingRow = (i64, Option<String>, i64);

/// The attempt rows of `chat`, oldest first.
pub(super) fn attempts_in(core: &BridgeCore, chat: &str) -> rusqlite::Result<Vec<Attempt>> {
    let db = lock(&core.db);
    let mut statement = db.prepare(
        "SELECT id,outcome,dispatched_at IS NOT NULL,boundary_pre_tokens,boundary_post_tokens,compaction_turn_id,
                trigger_turn_id,trigger_reading_id,trigger_used_tokens,next_reading_id,next_used_tokens,error,
                settled_at,reason,launch_started_at
         FROM early_compaction_attempts WHERE session_id=?1 ORDER BY created_at,rowid",
    )?;
    let rows = statement.query_map(params![chat], |row| {
        Ok(Attempt {
            id: row.get(0)?,
            outcome: row.get(1)?,
            dispatched: row.get(2)?,
            pre: row.get(3)?,
            post: row.get(4)?,
            compaction_turn_id: row.get(5)?,
            trigger_turn_id: row.get(6)?,
            trigger_reading_id: row.get(7)?,
            trigger_used_tokens: row.get(8)?,
            next_reading_id: row.get(9)?,
            next_used_tokens: row.get(10)?,
            error: row.get(11)?,
            settled_at: row.get(12)?,
            reason: row.get(13)?,
            launch_started_at: row.get(14)?,
        })
    })?;
    rows.collect()
}

pub(super) fn status_in(core: &BridgeCore, chat: &str) -> rusqlite::Result<String> {
    lock(&core.db).query_row("SELECT status FROM sessions WHERE id=?1", params![chat], |row| row.get(0))
}

/// What a hook needs to feed frames from inside an adapter call.
pub(super) struct Feeder {
    core: Weak<BridgeCore>,
    chat: String,
    turn: Arc<Mutex<Option<String>>>,
    launch: (String, String),
}

impl Feeder {
    pub(super) fn feed(&self, frames: &[Value]) {
        if let Some(core) = self.core.upgrade() {
            feed_frames(&core, &self.chat, &self.turn, (&self.launch.0, &self.launch.1), frames);
        }
    }
}

fn feed_frames(core: &Arc<BridgeCore>, chat: &str, turn: &Arc<Mutex<Option<String>>>, launch: (&str, &str), frames: &[Value]) {
    for frame in frames {
        handle_agent_value_timed(core, chat, turn, frame, None, Some(launch));
    }
}

/// Work that runs on another thread from inside an adapter call of a
/// compaction, so it starts while the sweep holds its reservation.
#[derive(Default)]
pub(super) struct Race {
    waited: Slot<bool>,
    thread: Slot<JoinHandle<bool>>,
}

impl Race {
    /// Start `work` in the next adapter call of a compaction of `world`.
    pub(super) fn arm(world: &World, work: impl FnOnce(Arc<BridgeCore>) -> bool + Send + 'static) -> Self {
        let race = Self::default();
        let (waited, thread, core) = (race.waited.clone(), race.thread.clone(), Arc::downgrade(&world.core));
        world.levers.on_compact(move || {
            let Some(core) = core.upgrade() else {
                return;
            };
            let (done, signal) = std::sync::mpsc::channel();
            let handle = std::thread::spawn(move || {
                let ok = work(core);
                let _receiver_gone = done.send(()).is_err();
                ok
            });
            *lock(&waited) = Some(matches!(signal.recv_timeout(WAIT), Err(RecvTimeoutError::Timeout)));
            *lock(&thread) = Some(handle);
        });
        race
    }

    /// Wait for the work. Returns whether it waited for the adapter call
    /// and whether it succeeded.
    pub(super) fn finish(&self) -> Outcome<(bool, bool)> {
        let handle = lock(&self.thread).take().ok_or("the adapter call of a compaction did not run")?;
        let ok = handle.join().map_err(|_| "the racing thread panicked")?;
        let waited = lock(&self.waited).take().ok_or("the race recorded no wait")?;
        Ok((waited, ok))
    }
}

/// How a turn in [`turn_frames`] ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Ending {
    Completed,
    /// The turn itself fails: an error subtype from Claude, a failed turn
    /// status from Codex.
    Failed,
    /// The provider reports an error: a Claude result with `is_error`, a
    /// Codex `error` that does not retry.
    ProviderError,
    /// The reply holds a prompt change fence.
    PromptControl,
}

/// A send through each public send path of the composer and of the client.
#[derive(Clone, Copy, Debug)]
pub(super) enum Variant {
    Legacy,
    Composer,
    Queue,
    Steer,
    Image,
}

pub(super) const VARIANTS: [Variant; 5] = [Variant::Legacy, Variant::Composer, Variant::Queue, Variant::Steer, Variant::Image];

/// A core with three warm direct chats that started an hour ago. The switch
/// is absent.
pub(super) struct World {
    _scratch: tempfile::TempDir,
    pub(super) core: Arc<BridgeCore>,
    pub(super) calls: CallLog,
    pub(super) levers: Levers,
    turns: HashMap<&'static str, Arc<Mutex<Option<String>>>>,
    serial: AtomicUsize,
}

impl World {
    pub(super) fn new() -> Outcome<Self> {
        let scratch = tempfile::tempdir()?;
        let mut core = BridgeCore::for_tests(scratch.path());
        let levers = Levers::default();
        let mut registry = AdapterRegistry::empty();
        for id in ["claude", "codex", "opencode"] {
            registry.register(Box::new(EarlyHarness { id, inner: TraceHarness::new(id), launches: levers.launches.clone() }))?;
        }
        core.adapter_registry = Arc::new(registry);
        let core = Arc::new(core);
        let calls = CallLog::default();
        let started_at = (Utc::now() - chrono::TimeDelta::hours(1)).to_rfc3339();
        let mut turns = HashMap::new();
        for (session, harness, provider) in CHATS {
            seed(&core, session, harness, provider, &started_at)?;
            let inner = TraceRuntime {
                session,
                provider,
                claude: harness == "claude",
                turn: Arc::default(),
                requests: AtomicUsize::new(1),
                calls: calls.clone(),
            };
            turns.insert(session, inner.current_turn());
            let runtime = EarlyRuntime { inner, levers: levers.clone(), core: Arc::downgrade(&core) };
            lock(&core.adapters).insert(session.to_owned(), Box::new(runtime));
        }
        Ok(Self { _scratch: scratch, core, calls, levers, turns, serial: AtomicUsize::new(1) })
    }

    /// A new core on the store of this one, as after a restart of Bridge. It
    /// reads the switch as the startup does and has a live runtime for each
    /// chat.
    pub(super) fn restart(&self) -> Outcome<Self> {
        let restarted = Self::new()?;
        let store = std::mem::replace(&mut *lock(&self.core.db), rusqlite::Connection::open_in_memory()?);
        let switch = crate::early_compaction_settings::stored_switch(&store);
        *lock(&restarted.core.db) = store;
        restarted.core.early_compaction.set_enabled(switch);
        Ok(restarted)
    }

    pub(super) fn switch(&self, enabled: bool) -> Result<(), BridgeError> {
        let params = SaveEarlyCompactionSettingsParams { session_id: None, enabled: Some(enabled) };
        crate::early_compaction_settings::save(&self.core, &params).map(|_| ())
    }

    /// Save the opt-out of `chat`, or remove it.
    pub(super) fn opt_out(&self, chat: &str, out: bool) -> Result<(), BridgeError> {
        let params = SaveEarlyCompactionSettingsParams { session_id: Some(chat.to_owned()), enabled: out.then_some(false) };
        crate::early_compaction_settings::save(&self.core, &params).map(|_| ())
    }

    pub(super) fn sql(&self, sql: &str, values: impl rusqlite::Params) -> rusqlite::Result<usize> {
        lock(&self.core.db).execute(sql, values)
    }

    pub(super) fn count(&self, sql: &str, values: impl rusqlite::Params) -> rusqlite::Result<i64> {
        lock(&self.core.db).query_row(sql, values, |row| row.get(0))
    }

    /// The current launch of `chat`: `started_at` and `provider_session_id`.
    pub(super) fn launch(&self, chat: &str) -> rusqlite::Result<(String, String)> {
        lock(&self.core.db).query_row(
            "SELECT started_at,provider_session_id FROM sessions WHERE id=?1",
            params![chat],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
    }

    /// A new launch of `chat` on the same thread, as after a relaunch.
    pub(super) fn relaunch(&self, chat: &str) -> rusqlite::Result<usize> {
        let started_at = (Utc::now() - chrono::TimeDelta::minutes(30)).to_rfc3339();
        self.sql("UPDATE sessions SET started_at=?2,status='ready',active_turn_id=NULL WHERE id=?1", params![chat, started_at])
    }

    pub(super) fn status(&self, chat: &str) -> rusqlite::Result<String> {
        status_in(&self.core, chat)
    }

    /// The harness went idle with no signal that Bridge saw.
    pub(super) fn idle(&self, chat: &str) -> rusqlite::Result<usize> {
        self.sql("UPDATE sessions SET status='ready',active_turn_id=NULL WHERE id=?1", params![chat])
    }

    pub(super) fn mark(&self, chat: &str) -> Option<DueMark> {
        self.core.early_compaction.due_mark(chat)
    }

    /// A due mark of the current launch, as a turn end of `chat` writes it.
    pub(super) fn inject_mark(&self, chat: &str, harness: &str) -> TestResult {
        let (started_at, provider_session_id) = self.launch(chat)?;
        self.core.early_compaction.accept(Receipt::Due(DueMark {
            session_id: chat.to_owned(),
            turn_id: None,
            completed_at: Utc::now(),
            launch: Some(crate::early_compaction::Launch { started_at, provider_session_id }),
            harness: harness.to_owned(),
        }));
        Ok(())
    }

    /// Feed `frames` through the reader of the current launch of `chat`.
    pub(super) fn feed(&self, chat: &str, frames: &[Value]) -> TestResult {
        let (started_at, provider) = self.launch(chat)?;
        self.feed_as(chat, (&started_at, &provider), frames)
    }

    pub(super) fn feed_as(&self, chat: &str, launch: (&str, &str), frames: &[Value]) -> TestResult {
        let turn = self.turns.get(chat).ok_or("no such chat")?;
        feed_frames(&self.core, chat, turn, launch, frames);
        Ok(())
    }

    pub(super) fn feeder(&self, chat: &str) -> Outcome<Feeder> {
        let turn = self.turns.get(chat).ok_or("no such chat")?.clone();
        Ok(Feeder { core: Arc::downgrade(&self.core), chat: chat.to_owned(), turn, launch: self.launch(chat)? })
    }

    pub(super) fn send(&self, chat: &str, text: &str) -> Result<wire::SubmitInputResult, BridgeError> {
        submit_input(&self.core, chat.to_owned(), text.to_owned())
    }

    /// One send of `FOLLOW_UP` through `variant`, as JSON: the result or the
    /// error text.
    pub(super) fn send_variant(&self, chat: &str, variant: Variant) -> Value {
        let (core, id, text) = (&self.core, chat.to_owned(), FOLLOW_UP.to_owned());
        let result = match variant {
            Variant::Legacy => send_turn(core, id, text).map(|()| Value::Null),
            Variant::Composer => submit_input(core, id, text).map(json_of),
            Variant::Queue => submit_input_with_preference(core, id, text, Vec::new(), Some(wire::ActiveTurnInput::Queue)).map(json_of),
            Variant::Steer => submit_input_with_preference(core, id, text, Vec::new(), Some(wire::ActiveTurnInput::Steer)).map(json_of),
            Variant::Image => submit_input_with_attachments(core, id, text, vec![png_image()]).map(json_of),
        };
        result.unwrap_or_else(|error| json!({"error": error.to_string()}))
    }

    pub(super) fn next_number(&self) -> usize {
        self.serial.fetch_add(1, Ordering::SeqCst)
    }

    /// Send a message and feed its whole turn, with a window reading of
    /// `used` tokens. Returns the turn id of the frames.
    pub(super) fn turn(&self, chat: &str, used: u64) -> Outcome<String> {
        self.turn_ending(chat, Ending::Completed, used)
    }

    pub(super) fn turn_ending(&self, chat: &str, ending: Ending, used: u64) -> Outcome<String> {
        self.send(chat, "Read notes.txt and summarise it.")?;
        let (turn, frames) = turn_frames(chat, self.next_number(), ending, used);
        self.feed(chat, &frames)?;
        Ok(turn)
    }

    /// The frames of a whole native compaction of `chat`. Returns the turn
    /// id of the Codex compaction.
    pub(super) fn compaction(&self, chat: &str) -> Outcome<String> {
        let number = self.next_number();
        let turn = format!("codex-turn-{number}");
        let frames = if chat == CLAUDE_CHAT {
            claude_compaction("manual")
        } else {
            codex_compaction(&turn, &format!("codex-compaction-{number}"))
        };
        self.feed(chat, &frames)?;
        Ok(turn)
    }

    /// A manual `/compact` and the frames of its compaction.
    pub(super) fn manual_compaction(&self, chat: &str) -> Outcome<String> {
        self.send(chat, "/compact")?;
        self.compaction(chat)
    }

    /// The end of the compaction turn with no boundary.
    pub(super) fn turn_end_without_boundary(&self, chat: &str) -> TestResult {
        let turn = format!("codex-turn-{}", self.next_number());
        let frames = if chat == CLAUDE_CHAT {
            vec![claude_frame(claude_result("", 0, claude_usage(0, 0, 0, 0), (1, 1, 1, 1, 0.001)))]
        } else {
            vec![
                codex("turn/started", json!({"threadId": CODEX_THREAD, "turn": codex_turn(&turn, "inProgress")})),
                codex_turn_completed(&turn, "completed"),
            ]
        };
        self.feed(chat, &frames)
    }

    /// The failed end of the compaction turn, with `error` as the text of
    /// the provider.
    pub(super) fn failed_turn_end(&self, chat: &str, error: &str) -> TestResult {
        let turn = format!("codex-turn-{}", self.next_number());
        let frames = if chat == CLAUDE_CHAT {
            let mut result = claude_result("", 0, claude_usage(0, 0, 0, 0), (1, 1, 1, 1, 0.001));
            result["is_error"] = json!(true);
            result["result"] = json!(error);
            vec![claude_frame(result)]
        } else {
            let mut failed = codex_turn(&turn, "failed");
            failed["error"] = json!({"message": error});
            vec![
                codex("turn/started", json!({"threadId": CODEX_THREAD, "turn": codex_turn(&turn, "inProgress")})),
                codex("turn/completed", json!({"threadId": CODEX_THREAD, "turn": failed})),
            ]
        };
        self.feed(chat, &frames)
    }

    pub(super) fn sweep_after(&self, seconds: i64) {
        crate::early_compaction::sweep(&self.core, Utc::now() + chrono::TimeDelta::seconds(seconds));
    }

    /// A sweep at the time a warm mark fires.
    pub(super) fn fire(&self) {
        self.sweep_after(WARM_SECONDS);
    }

    /// The focus of each compaction that reached the runtime of `chat`.
    pub(super) fn compacts(&self, chat: &str) -> Vec<Value> {
        lock(&self.calls)
            .iter()
            .filter(|call| call["call"] == "compact_native" && call["session"] == chat)
            .map(|call| call["args"].clone())
            .collect()
    }

    pub(super) fn take_calls(&self) -> Vec<Value> {
        std::mem::take(&mut *lock(&self.calls))
    }

    /// Where `text` went: the name of each call in `calls` that carries it to
    /// the runtime of `chat`, then the state of each queue row that holds it.
    pub(super) fn routes(&self, chat: &str, text: &str, calls: &[Value]) -> rusqlite::Result<Vec<String>> {
        let carries = |call: &&Value| {
            call["session"] == chat
                && call["args"].as_array().is_some_and(|args| args.iter().any(|arg| arg.as_str().is_some_and(|arg| arg.contains(text))))
        };
        let mut routes: Vec<String> = calls.iter().filter(carries).map(|call| call["call"].to_string()).collect();
        let db = lock(&self.core.db);
        let mut statement = db.prepare(
            "SELECT state FROM queued_session_input WHERE session_id=?1 AND instr(provider_text,?2)>0 ORDER BY sequence",
        )?;
        let rows = statement.query_map(params![chat, text], |row| row.get::<_, String>(0))?;
        for state in rows {
            routes.push(format!("queue {}", state?));
        }
        Ok(routes)
    }

    pub(super) fn attempts(&self, chat: &str) -> rusqlite::Result<Vec<Attempt>> {
        attempts_in(&self.core, chat)
    }

    /// The one attempt row of `chat`.
    pub(super) fn attempt(&self, chat: &str) -> Outcome<Attempt> {
        let mut rows = self.attempts(chat)?;
        if rows.len() != 1 {
            return Err(format!("{chat} has {} attempt rows", rows.len()).into());
        }
        rows.pop().ok_or_else(|| "no attempt row".into())
    }

    pub(super) fn readings(&self, chat: &str) -> rusqlite::Result<Vec<ReadingRow>> {
        let db = lock(&self.core.db);
        let mut statement = db.prepare("SELECT id,turn_id,used_tokens FROM context_readings WHERE session_id=?1 ORDER BY id")?;
        let rows = statement.query_map(params![chat], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        rows.collect()
    }

    pub(super) fn last_reading(&self, chat: &str) -> Outcome<ReadingRow> {
        self.readings(chat)?.pop().ok_or_else(|| format!("{chat} has no reading").into())
    }
}

fn json_of(result: wire::SubmitInputResult) -> Value {
    serde_json::to_value(result).unwrap_or_else(|error| json!({"serialize": error.to_string()}))
}

fn seed(core: &BridgeCore, session: &str, harness: &str, provider: &str, started_at: &str) -> rusqlite::Result<usize> {
    let model = match harness {
        "claude" => CLAUDE_MODEL,
        "codex" => CODEX_MODEL,
        _ => "opencode-model",
    };
    lock(&core.db).execute(
        "INSERT INTO sessions(id,workspace_id,harness,model,label,title,status,metric_source,kind,depth,provider_session_id,started_at)
         VALUES(?1,NULL,?2,?3,'Chat','Notes chat','ready','reported','direct',0,?4,?5)",
        params![session, harness, model, provider, started_at],
    )
}

/// The switch on and one warm mark on `chat` from a turn of `LARGE` tokens.
/// Returns the turn id too.
pub(super) fn marked(chat: &str) -> Outcome<(World, String)> {
    let world = World::new()?;
    world.switch(true)?;
    let turn = world.turn(chat, LARGE)?;
    if world.mark(chat).is_none() {
        return Err(format!("the turn of {chat} gave no due mark").into());
    }
    Ok((world, turn))
}

/// A world in which the sweep asked `chat` to compact once.
pub(super) fn fired(chat: &str) -> Outcome<World> {
    let (world, _) = marked(chat)?;
    world.fire();
    if world.compacts(chat).len() != 1 {
        return Err(format!("the sweep did not ask {chat} to compact").into());
    }
    Ok(world)
}

/// Whether a world with the switch on marks `chat` after one turn that ends
/// as `ending`.
pub(super) fn marked_after(chat: &str, ending: Ending) -> Outcome<bool> {
    let world = World::new()?;
    world.switch(true)?;
    world.turn_ending(chat, ending, LARGE)?;
    Ok(world.mark(chat).is_some())
}

/// The frames of one turn of `chat` and its turn id.
pub(super) fn turn_frames(chat: &str, number: usize, ending: Ending, used: u64) -> (String, Vec<Value>) {
    let text = if ending == Ending::PromptControl { CONTROL_REPLY } else { ANSWER };
    if chat == CLAUDE_CHAT {
        (format!("claude-request-{number}"), claude_frames(number, text, ending, used))
    } else {
        let turn = format!("codex-turn-{number}");
        let frames = codex_frames(&turn, text, ending, used);
        (turn, frames)
    }
}

/// A Claude turn with one reply. The window reading comes after the result,
/// as the sidecar reads it.
fn claude_frames(number: usize, text: &str, ending: Ending, used: u64) -> Vec<Value> {
    let usage = claude_usage(2, 1_000, 1_000, 50);
    let scale = u64::try_from(number).unwrap_or(1);
    let totals = (scale * 1_000, scale * 50, scale * 1_000, scale * 1_000, 0.001);
    let mut result = claude_result(text, 1, usage.clone(), totals);
    match ending {
        Ending::Failed => result["subtype"] = json!("error_during_execution"),
        Ending::ProviderError => {
            result["is_error"] = json!(true);
            result["result"] = json!("API Error: 529 overloaded");
        }
        Ending::Completed | Ending::PromptControl => {}
    }
    let mut frames = vec![
        claude_init(),
        claude_status(json!("requesting"), &format!("claude-request-{number}")),
        claude_assistant(&format!("msg_{number:02}"), json!([{"type": "text", "text": text}]), usage),
        result,
    ];
    if matches!(ending, Ending::Completed | Ending::PromptControl) {
        frames.push(claude_context_usage(used));
    }
    frames.into_iter().map(claude_frame).collect()
}

/// A Codex turn with one reply and a window reading of `used` tokens.
fn codex_frames(turn: &str, text: &str, ending: Ending, used: u64) -> Vec<Value> {
    let mut frames = vec![
        json!({"id": crate::codex_adapter::TURN_START_REQUEST_ID_BASE, "result": {"turn": codex_turn(turn, "inProgress")}}),
        codex("turn/started", json!({"threadId": CODEX_THREAD, "turn": codex_turn(turn, "inProgress")})),
    ];
    match ending {
        Ending::Failed => frames.push(codex_turn_completed(turn, "failed")),
        Ending::ProviderError => frames.push(codex(
            "error",
            json!({"error": {"message": "The model is overloaded."}, "willRetry": false, "threadId": CODEX_THREAD, "turnId": turn}),
        )),
        Ending::Completed | Ending::PromptControl => {
            let reply = json!({"type": "agentMessage", "id": format!("{turn}-reply"), "text": text, "phase": "final_answer"});
            frames.push(codex_item("item/completed", turn, reply));
            frames.push(codex_usage(turn, codex_tokens(used + 1_000, used + 1_000, 0, 0), codex_tokens(used, used, 0, 0)));
            frames.push(codex_turn_completed(turn, "completed"));
        }
    }
    frames
}

/// The boundary of a Claude `/compact`. `post` is absent when Claude Code
/// reports no figure after the compaction.
pub(super) fn claude_boundary(post: Option<i64>) -> Value {
    let mut metadata = json!({"trigger": "manual", "pre_tokens": PRE_TOKENS, "duration_ms": 5_705});
    if let Some(post) = post {
        metadata["post_tokens"] = json!(post);
    }
    claude_frame(json!({"type": "system", "subtype": "compact_boundary", "uuid": "claude-boundary", "compact_metadata": metadata}))
}

/// What Claude Code sends after a boundary: a `result` with a usage of zero
/// and the window reading.
pub(super) fn claude_compaction_end(used: u64) -> Vec<Value> {
    vec![
        claude_result("", 0, claude_usage(0, 0, 0, 0), (15_619, 1_765, 76_248, 33_818, 0.009)),
        claude_context_usage(used),
    ]
    .into_iter()
    .map(claude_frame)
    .collect()
}

/// The frames of a Claude compaction up to the boundary.
pub(super) fn claude_compaction_start() -> Vec<Value> {
    vec![
        claude_status(json!("compacting"), "claude-compacting"),
        json!({"type": "system", "subtype": "status", "status": null, "compact_result": "success", "uuid": "claude-compacted"}),
        claude_init(),
    ]
    .into_iter()
    .map(claude_frame)
    .collect()
}

/// A whole Claude compaction with the boundary figures and the window after.
pub(super) fn claude_compaction_with(post: Option<i64>, used: u64) -> Vec<Value> {
    let mut frames = claude_compaction_start();
    frames.push(claude_boundary(post));
    frames.extend(claude_compaction_end(used));
    frames
}

/// A Claude window reading frame.
pub(super) fn claude_reading(used: u64) -> Value {
    claude_frame(claude_context_usage(used))
}

/// The frames of `thread/compact/start` from the reply to the turn end.
pub(super) fn codex_compaction(turn: &str, item: &str) -> Vec<Value> {
    let mut frames = codex_compaction_start(3, turn, item);
    frames.extend(codex_compaction_end(turn, item));
    frames
}

/// The completed compaction item of a Codex turn: the Codex boundary.
pub(super) fn codex_boundary(turn: &str, item: &str) -> Value {
    codex_item("item/completed", turn, json!({"type": "contextCompaction", "id": item}))
}

/// A Codex window reading of `used` tokens in `turn`.
pub(super) fn codex_reading(turn: &str, used: u64) -> Value {
    codex_usage(turn, codex_tokens(used + 1_000, used + 1_000, 0, 0), codex_tokens(used, used, 0, 0))
}

/// A Codex turn in which Codex compacts on its own before it replies.
pub(super) fn codex_turn_with_auto_compaction(turn: &str, used: u64) -> Vec<Value> {
    let item = json!({"type": "contextCompaction", "id": format!("{turn}-compaction")});
    let mut frames = codex_frames(turn, ANSWER, Ending::Completed, used);
    frames.insert(2, codex_item("item/started", turn, item.clone()));
    frames.insert(3, codex_item("item/completed", turn, item));
    frames
}

/// The statements of each traced connection, with the thread that ran each
/// one. Each test keeps only the statements of its own thread.
static TRACE: Mutex<Vec<(ThreadId, String)>> = Mutex::new(Vec::new());

fn trace_into(statement: &str) {
    lock(&TRACE).push((std::thread::current().id(), statement.to_owned()));
}

/// The statements that `work` runs on the database of `core` from this
/// thread.
pub(super) fn statements<T>(core: &BridgeCore, work: impl FnOnce() -> T) -> (T, Vec<String>) {
    lock(&core.db).trace(Some(trace_into));
    let result = work();
    lock(&core.db).trace(None);
    let me = std::thread::current().id();
    let mut traced = lock(&TRACE);
    let (mine, others): (Vec<_>, Vec<_>) = std::mem::take(&mut *traced).into_iter().partition(|(thread, _)| *thread == me);
    *traced = others;
    (result, mine.into_iter().map(|(_, statement)| statement).collect())
}

/// The statements with each literal replaced, so two runs compare by shape.
pub(super) fn shapes(statements: &[String]) -> Result<Vec<String>, regex::Error> {
    let literal = Regex::new(r"'(?:[^']|'')*'|\b\d+(?:\.\d+)?\b")?;
    Ok(statements.iter().map(|statement| literal.replace_all(statement, "?").into_owned()).collect())
}

/// Replace each generated id, so two worlds compare by value.
pub(super) fn without_ids(value: &Value) -> Result<String, regex::Error> {
    let uuid = Regex::new(r"(?i)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")?;
    Ok(uuid.replace_all(&value.to_string(), "<id>").into_owned())
}

pub(super) fn png_image() -> TurnImage {
    TurnImage { media_type: "image/png".into(), base64_data: "iVBORw0".into() }
}

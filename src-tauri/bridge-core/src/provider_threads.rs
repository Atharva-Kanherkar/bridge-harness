//! The provider threads a chat leaves behind when it switches app or account,
//! so switching back resumes that thread instead of starting a fresh one.
//!
//! A switch to another harness clears `sessions.provider_session_id`, and the
//! next start projects stored history into a brand-new thread. That is the
//! only option for an app the chat never ran on. But a chat that goes Claude,
//! Codex, Claude still has its first Claude thread on disk, with every tool
//! call and file read the projection leaves out. This table remembers it, per
//! continuation key (the app plus the home its account reads threads from),
//! with the last forest sequence the thread saw.
//!
//! Switching back plans a native continuation onto the remembered thread. The
//! thread already holds everything up to `left_at_sequence`, so the first
//! start after the return owes it only what was said since then: the delta
//! rides once in the session-context frame beside the next message, never in
//! the system prompt, so the resumed thread's prefix stays as it was.

use crate::{
    model::SessionEntry, provider_accounts, restoration, session_forest::SessionForest,
    BridgeError,
};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction};

/// Schema 86.
pub(crate) fn install_store(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_provider_threads (
            session_id TEXT NOT NULL,
            harness TEXT NOT NULL,
            account TEXT,
            continuation_key TEXT NOT NULL,
            provider_session_id TEXT NOT NULL,
            left_at_sequence INTEGER NOT NULL,
            resume_after_sequence INTEGER,
            updated_at TEXT NOT NULL,
            PRIMARY KEY(session_id, continuation_key)
        );",
    )?;
    Ok(())
}

/// The key a thread is remembered and found under: the harness plus the home
/// its account reads threads from. `None` when the account no longer exists,
/// since a removed account shares nothing.
pub fn continuation_key(db: &Connection, harness: &str, account: Option<&str>) -> Option<String> {
    let record = match account {
        Some(id) => Some(crate::agent_config::account(db, harness, id).ok()?),
        None => None,
    };
    let host = provider_accounts::HostHomes::from_env();
    Some(
        provider_accounts::continuation_key(harness, record.as_ref(), &host)
            .unwrap_or_else(|| format!("{harness}:default")),
    )
}

/// A remembered thread for one continuation key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeftThread {
    pub provider_session_id: String,
    /// The newest forest sequence the thread had seen when the chat left it.
    pub left_at_sequence: i64,
}

/// Remember that the chat is leaving `provider_session_id`. Replaces whatever
/// was remembered under the same key: the thread being left is the newest.
pub fn record_left(
    db: &Connection,
    session_id: &str,
    harness: &str,
    account: Option<&str>,
    continuation_key: &str,
    provider_session_id: &str,
) -> Result<(), BridgeError> {
    let left_at_sequence: i64 = db.query_row(
        "SELECT COALESCE(MAX(sequence),0) FROM session_entries WHERE session_id=?1",
        params![session_id],
        |row| row.get(0),
    )?;
    db.execute(
        "INSERT INTO session_provider_threads(session_id,harness,account,continuation_key,provider_session_id,left_at_sequence,resume_after_sequence,updated_at)
         VALUES(?1,?2,?3,?4,?5,?6,NULL,?7)
         ON CONFLICT(session_id,continuation_key) DO UPDATE SET
            harness=excluded.harness,
            account=excluded.account,
            provider_session_id=excluded.provider_session_id,
            left_at_sequence=excluded.left_at_sequence,
            resume_after_sequence=NULL,
            updated_at=excluded.updated_at",
        params![
            session_id,
            harness,
            account,
            continuation_key,
            provider_session_id,
            left_at_sequence,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(())
}

/// The thread remembered for this chat under `continuation_key`, if any.
pub fn find(
    db: &Connection,
    session_id: &str,
    continuation_key: &str,
) -> Result<Option<LeftThread>, BridgeError> {
    Ok(db
        .query_row(
            "SELECT provider_session_id,left_at_sequence FROM session_provider_threads WHERE session_id=?1 AND continuation_key=?2",
            params![session_id, continuation_key],
            |row| {
                Ok(LeftThread {
                    provider_session_id: row.get(0)?,
                    left_at_sequence: row.get(1)?,
                })
            },
        )
        .optional()?)
}

/// A switch is going back to the thread under `continuation_key`: owe its
/// next native start everything said since it was left.
pub fn mark_returned(
    db: &Connection,
    session_id: &str,
    continuation_key: &str,
) -> Result<(), BridgeError> {
    db.execute(
        "UPDATE session_provider_threads SET resume_after_sequence=left_at_sequence,updated_at=?3 WHERE session_id=?1 AND continuation_key=?2",
        params![session_id, continuation_key, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

/// The forest sequence after which `provider_session_id` is owed the
/// conversation, when a return to it has not been settled yet.
pub fn owed_after(
    db: &Connection,
    session_id: &str,
    provider_session_id: &str,
) -> Result<Option<i64>, BridgeError> {
    Ok(db
        .query_row(
            "SELECT resume_after_sequence FROM session_provider_threads WHERE session_id=?1 AND provider_session_id=?2 AND resume_after_sequence IS NOT NULL",
            params![session_id, provider_session_id],
            |row| row.get(0),
        )
        .optional()?)
}

/// How many conversation entries the active branch holds after `sequence`.
pub fn entries_since(db: &Connection, session_id: &str, sequence: i64) -> Result<usize, BridgeError> {
    Ok(conversation_since(db, session_id, sequence)?
        .iter()
        .filter(|entry| is_said(entry))
        .count())
}

/// A conversation entry the delta lists as said: not a summary of it.
fn is_said(entry: &SessionEntry) -> bool {
    // A switch checkpoint landing in this window is the returning thread's own
    // summary of itself, or the other model's summary of turns already listed.
    matches!(
        entry.kind.as_str(),
        "user.message" | "assistant.message" | "worker.result" | "handoff.brief"
    ) && restoration::entry_text(entry).is_some()
}

/// The active branch after `sequence`: what was said, and the tool calls the
/// other app made in between (their started entries too, which name what a
/// Claude tool result ran).
fn conversation_since(
    db: &Connection,
    session_id: &str,
    sequence: i64,
) -> Result<Vec<SessionEntry>, BridgeError> {
    let branch = SessionForest::new(db)
        .active_branch(session_id)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    Ok(branch
        .into_iter()
        .filter(|entry| entry.sequence > sequence)
        .filter(|entry| {
            is_said(entry)
                || crate::tool_facts::is_completed_tool(&entry.kind)
                || crate::tool_facts::is_started_tool(&entry.kind)
        })
        .collect())
}

/// What `provider_session_id` is owed on its first native start after a
/// switch came back to it: the conversation since it was left, oldest first,
/// within the restoration budget of a model with `context_window_tokens`.
/// `None` when nothing is owed, or nothing was said in between.
pub fn owed_delta(
    db: &Connection,
    session_id: &str,
    provider_session_id: &str,
    context_window_tokens: i64,
) -> Result<Option<String>, BridgeError> {
    let Some(after) = owed_after(db, session_id, provider_session_id)? else {
        return Ok(None);
    };
    let entries = conversation_since(db, session_id, after)?;
    let refs = entries.iter().collect::<Vec<_>>();
    let budget = restoration::restoration_budget_bytes(context_window_tokens);
    let tail = restoration::render_tail(&refs, budget, restoration::restoration_entry_bytes(budget));
    if tail.lines.is_empty() {
        return Ok(None);
    }
    let said = entries.iter().filter(|entry| is_said(entry)).count();
    let omitted = said.saturating_sub(tail.messages);
    let lines = tail.lines;
    let mut text = String::from(
        "You are back on this conversation after it ran on another model for a while. \
         Your own thread above is unchanged; this is what was said in this chat since \
         your last turn here, and what the other model's tools did, oldest first. Files \
         may have changed since then.",
    );
    if omitted > 0 {
        text.push_str(&format!(" The {omitted} oldest of these entries did not fit and are left out."));
    }
    for line in lines {
        text.push('\n');
        text.push_str(&line);
    }
    Ok(Some(text))
}

/// The return to `provider_session_id` is done: its native start carried the
/// delta (or had none to carry). It is owed nothing more.
pub fn settle(db: &Connection, session_id: &str, provider_session_id: &str) -> Result<(), BridgeError> {
    db.execute(
        "UPDATE session_provider_threads SET resume_after_sequence=NULL,updated_at=?3 WHERE session_id=?1 AND provider_session_id=?2",
        params![session_id, provider_session_id, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

/// `provider_session_id` could not be resumed: forget it, so no later switch
/// plans a return onto a thread that is gone.
pub fn forget(db: &Connection, session_id: &str, provider_session_id: &str) -> Result<(), BridgeError> {
    db.execute(
        "DELETE FROM session_provider_threads WHERE session_id=?1 AND provider_session_id=?2",
        params![session_id, provider_session_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_forest::EntryKind;
    use serde_json::json;

    fn db() -> Connection {
        let db = crate::store::open(std::path::Path::new(":memory:")).unwrap();
        db.execute(
            "INSERT INTO sessions(id,harness,label,status,metric_source) VALUES('s','claude','Chat','idle','reported')",
            [],
        )
        .unwrap();
        db
    }

    fn say(db: &Connection, kind: EntryKind, text: &str) {
        SessionForest::new(db).append("s", kind, json!({"text": text})).unwrap();
    }

    #[test]
    fn a_left_thread_is_found_by_its_key_and_replaced_by_a_newer_one() {
        let db = db();
        say(&db, EntryKind::UserMessage, "first");
        record_left(&db, "s", "claude", None, "claude:home:/h", "thread-1").unwrap();
        let found = find(&db, "s", "claude:home:/h").unwrap().unwrap();
        assert_eq!(found.provider_session_id, "thread-1");
        assert!(found.left_at_sequence > 0);
        assert!(find(&db, "s", "claude:home:/other").unwrap().is_none(), "another home shares nothing");

        say(&db, EntryKind::AssistantMessage, "second");
        record_left(&db, "s", "claude", None, "claude:home:/h", "thread-2").unwrap();
        let newer = find(&db, "s", "claude:home:/h").unwrap().unwrap();
        assert_eq!(newer.provider_session_id, "thread-2");
        assert!(newer.left_at_sequence > found.left_at_sequence);
    }

    #[test]
    fn a_return_owes_only_what_was_said_since_and_only_once() {
        let db = db();
        say(&db, EntryKind::UserMessage, "already in the thread");
        record_left(&db, "s", "claude", None, "k", "thread-1").unwrap();
        assert!(owed_delta(&db, "s", "thread-1", 200_000).unwrap().is_none(), "nothing owed before a return");

        say(&db, EntryKind::UserMessage, "asked the other model");
        say(&db, EntryKind::AssistantMessage, "the other model answered");
        mark_returned(&db, "s", "k").unwrap();
        assert_eq!(entries_since(&db, "s", find(&db, "s", "k").unwrap().unwrap().left_at_sequence).unwrap(), 2);

        let delta = owed_delta(&db, "s", "thread-1", 200_000).unwrap().unwrap();
        assert!(delta.contains("user.message: asked the other model"), "{delta}");
        assert!(delta.contains("assistant.message: the other model answered"), "{delta}");
        assert!(!delta.contains("already in the thread"), "the thread already holds it: {delta}");
        assert!(
            delta.find("asked the other model").unwrap() < delta.find("the other model answered").unwrap(),
            "oldest first"
        );

        settle(&db, "s", "thread-1").unwrap();
        assert!(owed_delta(&db, "s", "thread-1", 200_000).unwrap().is_none(), "owed once");
    }

    #[test]
    fn the_delta_for_a_returned_thread_carries_tool_lines() {
        let db = db();
        say(&db, EntryKind::UserMessage, "already in the thread");
        record_left(&db, "s", "claude", None, "k", "thread-1").unwrap();
        say(&db, EntryKind::UserMessage, "fix the flaky test on the other model");
        crate::restoration::tests::tool_event(&db, "s", "command.completed", "exec-1", None, Some("OUTPUT"), json!({"command": "/bin/zsh -lc 'cargo test retry'", "exitCode": 0}));
        crate::restoration::tests::tool_event(&db, "s", "file_change.completed", "exec-2", None, None, json!({"changes": [{"path": "src/retry.rs"}]}));
        say(&db, EntryKind::AssistantMessage, "fixed and green");
        mark_returned(&db, "s", "k").unwrap();
        assert_eq!(entries_since(&db, "s", find(&db, "s", "k").unwrap().unwrap().left_at_sequence).unwrap(), 2, "tools are not messages");
        let delta = owed_delta(&db, "s", "thread-1", 200_000).unwrap().unwrap();
        let tools = delta.find("tools: ran `cargo test retry` (exit 0); edited src/retry.rs").unwrap_or_else(|| panic!("{delta}"));
        assert!(delta.find("fix the flaky test").unwrap() < tools && tools < delta.find("fixed and green").unwrap(), "{delta}");
        assert!(!delta.contains("OUTPUT") && !delta.contains("did not fit"), "{delta}");
    }

    #[test]
    fn a_return_with_nothing_said_in_between_owes_nothing() {
        let db = db();
        say(&db, EntryKind::UserMessage, "hello");
        record_left(&db, "s", "claude", None, "k", "thread-1").unwrap();
        mark_returned(&db, "s", "k").unwrap();
        assert!(owed_delta(&db, "s", "thread-1", 200_000).unwrap().is_none());
    }

    #[test]
    fn a_thread_that_cannot_be_resumed_is_forgotten() {
        let db = db();
        record_left(&db, "s", "claude", None, "k", "thread-1").unwrap();
        mark_returned(&db, "s", "k").unwrap();
        forget(&db, "s", "thread-1").unwrap();
        assert!(find(&db, "s", "k").unwrap().is_none());
        assert!(owed_after(&db, "s", "thread-1").unwrap().is_none());
    }

    #[test]
    fn an_unknown_account_has_no_key_and_a_driverless_harness_still_has_one() {
        let db = db();
        assert!(continuation_key(&db, "claude", Some("no-such-account")).is_none());
        assert!(continuation_key(&db, "claude", None).unwrap().starts_with("claude:"));
    }
}

/// The launch half: `start_chat` on a thread a switch came back to.
#[cfg(test)]
mod launch_tests {
    use super::*;
    use crate::{
        adapters::{AdapterRegistry, AdapterRuntime, HarnessAdapter, ResumeRequest, ShutdownReason, StartRequest, StartedAdapter},
        agent::NormalizedEvent,
        model::{AdapterDescriptor, Harness, RestorationMode, ResumeEligibility},
        runtime::BridgeCore,
        session_forest::EntryKind,
    };
    use serde_json::{json, Value};
    use std::{
        io::{BufReader, Read},
        sync::{mpsc, Arc, Mutex},
    };

    /// A reader that blocks until its sender is dropped, so the launched
    /// session stays up for the assertions.
    struct Held(mpsc::Receiver<Vec<u8>>);
    impl Read for Held {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            // Nothing is ever sent: this returns once the sender is dropped.
            while self.0.recv().is_ok() {}
            Ok(0)
        }
    }

    struct Runtime(String);
    impl AdapterRuntime for Runtime {
        fn process_id(&self) -> u32 {
            std::process::id()
        }
        fn provider_session_id(&self) -> &str {
            &self.0
        }
        fn current_turn(&self) -> Arc<Mutex<Option<String>>> {
            Arc::new(Mutex::new(None))
        }
        fn send_turn(&self, _: &str) -> Result<(), BridgeError> {
            Ok(())
        }
        fn interrupt(&self) -> Result<(), BridgeError> {
            Ok(())
        }
        fn respond(&self, _: Value, _: &str) -> Result<(), BridgeError> {
            Ok(())
        }
        fn read_usage(&self) -> Result<(), BridgeError> {
            Ok(())
        }
        fn stop(&mut self, _: ShutdownReason) {}
    }

    #[derive(Default)]
    struct Calls {
        resumed: Vec<String>,
        started_with: Vec<String>,
    }

    struct ClaudeStub {
        calls: Arc<Mutex<Calls>>,
        resume_fails: bool,
        hold: Mutex<Vec<mpsc::Sender<Vec<u8>>>>,
    }
    impl ClaudeStub {
        fn started(&self, thread: &str) -> StartedAdapter {
            let (sender, receiver) = mpsc::channel();
            self.hold.lock().unwrap().push(sender);
            StartedAdapter {
                runtime: Box::new(Runtime(thread.to_owned())),
                reader: Box::new(BufReader::new(Held(receiver))),
                startup_messages: Vec::new(),
                skills_dropped: false,
            }
        }
    }
    impl HarnessAdapter for ClaudeStub {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn descriptor(&self) -> AdapterDescriptor {
            AdapterDescriptor {
                sandbox_modes: crate::model::SandboxMode::ALL.to_vec(),
                id: "claude".into(),
                label: "Claude".into(),
                available: true,
                auth_state: crate::model::AuthState::Unknown,
                version: None,
                capabilities: Vec::new(),
                unavailable_reason: None,
                models: Vec::new(),
                default_model: None,
                model_catalog: crate::model::ModelCatalogDiagnostics::curated(),
            }
        }
        fn start(&self, request: StartRequest<'_>) -> Result<StartedAdapter, BridgeError> {
            self.calls
                .lock()
                .unwrap()
                .started_with
                .push(request.instructions.unwrap_or_default().to_owned());
            Ok(self.started("fresh-thread"))
        }
        fn resume(&self, request: ResumeRequest<'_>) -> Result<StartedAdapter, BridgeError> {
            self.calls.lock().unwrap().resumed.push(request.provider_session_id.to_owned());
            if self.resume_fails {
                return Err(BridgeError::Adapter("no such session".into()));
            }
            Ok(self.started(request.provider_session_id))
        }
        fn supports_native_resume(&self) -> bool {
            true
        }
        fn normalize(&self, _: &Value) -> Vec<NormalizedEvent> {
            Vec::new()
        }
    }

    /// A Claude chat that left `claude-thread` for another app, where one
    /// exchange happened, and has just been switched back to it.
    fn returned_chat(resume_fails: bool) -> (Arc<BridgeCore>, String, Arc<Mutex<Calls>>) {
        let scratch = Box::leak(Box::new(tempfile::tempdir().unwrap()));
        let mut core = BridgeCore::for_tests(scratch.path());
        let calls = Arc::new(Mutex::new(Calls::default()));
        let mut registry = AdapterRegistry::empty();
        registry
            .register(Box::new(ClaudeStub { calls: calls.clone(), resume_fails, hold: Mutex::new(Vec::new()) }))
            .unwrap();
        core.adapter_registry = Arc::new(registry);
        let core = Arc::new(core);
        let session_id = core.create_chat_id(&Harness::Claude, None, None).unwrap();
        {
            let db = core.db.lock().unwrap();
            let forest = SessionForest::new(&db);
            forest.append(&session_id, EntryKind::UserMessage, json!({"text":"set up the experiment"})).unwrap();
            record_left(&db, &session_id, "claude", None, "claude:test", "claude-thread").unwrap();
            forest.append(&session_id, EntryKind::UserMessage, json!({"text":"asked codex while away"})).unwrap();
            forest.append(&session_id, EntryKind::AssistantMessage, json!({"text":"codex found the leak"})).unwrap();
            mark_returned(&db, &session_id, "claude:test").unwrap();
            db.execute("UPDATE sessions SET provider_session_id='claude-thread' WHERE id=?1", params![session_id]).unwrap();
            restoration::set_head_state(&db, &session_id, RestorationMode::Native, ResumeEligibility::Native, Some("claude-thread")).unwrap();
        }
        (core, session_id, calls)
    }

    fn ledger(core: &BridgeCore, session_id: &str) -> (String, String) {
        core.db
            .lock()
            .unwrap()
            .query_row(
                "SELECT restoration_mode,COALESCE(sections_json,'') FROM prompt_compilations WHERE session_id=?1 ORDER BY id DESC LIMIT 1",
                params![session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
    }

    #[test]
    fn the_returned_thread_resumes_and_is_handed_what_it_missed_once() {
        let (core, session_id, calls) = returned_chat(false);
        crate::live_turn::start_chat(&core, session_id.clone()).unwrap();
        assert_eq!(calls.lock().unwrap().resumed, vec!["claude-thread".to_owned()]);
        assert!(calls.lock().unwrap().started_with.is_empty(), "no fresh thread");
        let frame = core.session_context.lock().unwrap().pending(&session_id).expect("a frame is owed");
        let text = frame.text();
        assert!(text.contains("conversation_since_you_left"), "{text}");
        assert!(text.contains("asked codex while away") && text.contains("codex found the leak"), "{text}");
        assert!(!text.contains("set up the experiment"), "the thread already holds it: {text}");
        let db = core.db.lock().unwrap();
        assert!(owed_after(&db, &session_id, "claude-thread").unwrap().is_none(), "settled: owed once");
        assert!(find(&db, &session_id, "claude:test").unwrap().is_some(), "still remembered");
        drop(db);
        assert_eq!(ledger(&core, &session_id).0, "native");
    }

    #[test]
    fn a_returned_thread_that_will_not_resume_is_forgotten_and_history_is_projected() {
        let (core, session_id, calls) = returned_chat(true);
        crate::live_turn::start_chat(&core, session_id.clone()).unwrap();
        assert_eq!(calls.lock().unwrap().resumed, vec!["claude-thread".to_owned()]);
        let started_with = calls.lock().unwrap().started_with.clone();
        assert_eq!(started_with.len(), 1, "fell back to a fresh thread");
        assert!(started_with[0].contains("set up the experiment"), "the projection carries everything");
        assert!(started_with[0].contains("codex found the leak"));
        let frame = core.session_context.lock().unwrap().pending(&session_id);
        assert!(
            frame.as_ref().is_none_or(|frame| !frame.text().contains("conversation_since_you_left")),
            "the projection already covers the delta"
        );
        let db = core.db.lock().unwrap();
        assert!(find(&db, &session_id, "claude:test").unwrap().is_none(), "a dead thread is forgotten");
        drop(db);
        // The ledger records the prompt that carried the handoff.
        let (mode, sections) = ledger(&core, &session_id);
        assert_eq!(mode, "checkpoint_restored");
        assert!(sections.contains("restoration_context"), "{sections}");
    }
}

//! The production title run: the loop that drains `session_title_jobs`, and the
//! backend that reads Claude's transcript and runs one hidden turn per batch.
//!
//! The hidden turn has no tools. Claude runs under a briefing policy with an
//! empty scope, as memory extraction does; Codex and OpenCode start through
//! their text-completion path, read-only in an empty scratch directory, as the
//! composer's suggestions do. Any tool attempt ends the turn as a failure. No
//! `sessions` row is created, so a title run never appears in a list; its spend
//! is recorded on the job rows instead.

use std::io::BufRead;
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::time::{Duration as StdDuration, Instant};

use bridge_protocol::messages as wire;
use chrono::Utc;
use serde_json::Value;
use uuid::Uuid;

use crate::adapters::{AdapterRegistry, ShutdownReason, StartRequest};
use crate::briefing_policy::BriefingRuntimePolicy;
use crate::delegation::WriteMode;
use crate::events::CoreEvent;
use crate::runtime::BridgeCore;
use crate::session_titles;
use crate::title_jobs::{self, Candidate, Generation, TitleBackend};

/// How often an idle loop looks for due jobs. A turn-queued job waits at most
/// this long; the query is one indexed probe.
const POLL: StdDuration = StdDuration::from_secs(5);
/// Between batches while a backlog drains, so a backfill of hundreds of chats
/// never runs turns back to back against a shared account limit.
const BATCH_PAUSE: StdDuration = StdDuration::from_secs(3);
const MAX_WALL_SECONDS: i64 = 120;
/// Every title run shares one scratch directory, so Claude files them all under
/// one project instead of minting a directory per run.
const SCRATCH: &str = "title-runs";

/// Started once per host beside the other maintenance loops.
pub fn start_title_maintenance(core: Arc<BridgeCore>) {
    let _ = std::thread::Builder::new().name("title-jobs".into()).spawn(move || {
        let owner = format!("titles-{}", Uuid::new_v4());
        let backend = LiveBackend::new(core.adapter_registry.clone(), core.chat_scratch_dir(SCRATCH));
        loop {
            match title_jobs::drain_once(&core.db, &backend, &owner, Utc::now) {
                Ok(report) => {
                    if report.renamed > 0 {
                        core.events.publish(CoreEvent::StateChanged);
                    }
                    if report.claimed > 0 {
                        std::thread::sleep(BATCH_PAUSE);
                        continue;
                    }
                }
                Err(error) => eprintln!("[title-jobs] drain failed: {error}"),
            }
            std::thread::sleep(POLL);
        }
    });
}

/// Holds the registry rather than the whole core, so a harness that only has a
/// store and adapters (the end-to-end check below) can drive it too.
pub struct LiveBackend {
    registry: Arc<AdapterRegistry>,
    scratch: PathBuf,
}

impl LiveBackend {
    pub fn new(registry: Arc<AdapterRegistry>, scratch: PathBuf) -> Self {
        Self { registry, scratch }
    }
}

impl TitleBackend for LiveBackend {
    fn provider_title(&self, candidate: &Candidate) -> Option<String> {
        session_titles::provider_title(
            &candidate.harness,
            candidate.provider_session_id.as_deref(),
            candidate.cwd.as_deref(),
            &candidate.grounding,
        )
    }

    fn can_generate(&self, harness: &str) -> bool {
        self.registry.descriptor(harness).is_some_and(|descriptor| descriptor.available)
    }

    fn generate(&self, harness: &str, model: Option<&str>, instructions: &str, prompt: &str) -> Result<Generation, String> {
        one_hidden_turn(&self.registry, &self.scratch, harness, model, instructions, prompt)
    }
}

fn one_hidden_turn(
    registry: &AdapterRegistry,
    scratch: &std::path::Path,
    harness: &str,
    model: Option<&str>,
    instructions: &str,
    prompt: &str,
) -> Result<Generation, String> {
    std::fs::create_dir_all(scratch).map_err(|error| error.to_string())?;
    let cwd = scratch.to_string_lossy().to_string();
    let limits = wire::WorkBriefLimits {
        max_wall_seconds: MAX_WALL_SECONDS,
        max_turns: 1,
        max_tool_calls: 1,
        max_output_tokens: None,
        cost_ceiling_microusd: None,
    };
    let policy = BriefingRuntimePolicy::compile_scoped(Vec::new(), limits)
        .map_err(|unsupported| format!("policy: {}", unsupported.reason()))?;
    let started = if harness == "claude" {
        registry.start(
            harness,
            StartRequest {
                cwd: &cwd,
                model,
                effort: None,
                instructions: Some(instructions),
                write_mode: None,
                read_only_sandbox: None,
                briefing: Some(&policy),
                interactive: false,
                on_progress: None,
            },
        )
    } else {
        registry.start_completion(
            harness,
            StartRequest {
                cwd: &cwd,
                model,
                // a title needs no deliberation; harnesses without the knob drop it.
                effort: Some("low"),
                instructions: Some(instructions),
                // No file or shell access: this session only ever names chats.
                write_mode: Some(WriteMode::ReadOnly),
                read_only_sandbox: None,
                briefing: None,
                interactive: false,
                on_progress: None,
            },
        )
    };
    let started = started.map_err(|error| format!("provider start: {error}"))?;
    let mut runtime = started.runtime;

    let (lines, receiver) = mpsc::channel::<String>();
    let mut reader = started.reader;
    std::thread::spawn(move || {
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if lines.send(line.trim_end().to_owned()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    if let Err(error) = runtime.send_turn(prompt) {
        runtime.stop(ShutdownReason::Failed);
        return Err(format!("send: {error}"));
    }

    let deadline = Instant::now() + StdDuration::from_secs(MAX_WALL_SECONDS as u64);
    let mut turn = TurnText::default();
    let outcome = loop {
        if Instant::now() >= deadline {
            break Err("deadline exceeded".to_string());
        }
        match receiver.recv_timeout(StdDuration::from_secs(1)) {
            Ok(line) => {
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if let Some(done) = turn.observe(registry.normalize(harness, &value)) {
                    break done;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break Err("provider ended before a result".to_string()),
        }
    };
    runtime.stop(if outcome.is_ok() { ShutdownReason::Completed } else { ShutdownReason::Failed });
    outcome.map(|()| turn.generation())
}

/// One hidden turn's assistant text and usage, read from normalized events so
/// every harness is read the same way.
#[derive(Default)]
struct TurnText {
    parts: Vec<(Option<String>, String)>,
    started: bool,
    tokens: i64,
    spend_microusd: i64,
}

impl TurnText {
    /// Some once the turn is over: Ok at its end, Err on a tool attempt or error.
    fn observe(&mut self, events: Vec<crate::agent::NormalizedEvent>) -> Option<Result<(), String>> {
        let mut finished = None;
        // Consume the whole batch: a turn's usage can arrive beside its end.
        for event in events {
            match event.kind.as_str() {
                "turn.started" => self.started = true,
                "message.delta" | "message.completed" if event.role.as_deref() == Some("assistant") => {
                    self.started = true;
                    let Some(text) = event.text else { continue };
                    match self.parts.iter_mut().find(|(id, _)| *id == event.item_id) {
                        Some((_, part)) if event.kind == "message.completed" => *part = text,
                        Some((_, part)) => part.push_str(&text),
                        None => self.parts.push((event.item_id, text)),
                    }
                }
                "usage.updated" => {
                    let count = |key: &str| event.data.pointer(&format!("/usage/{key}")).and_then(Value::as_i64).unwrap_or(0);
                    self.tokens += count("input_tokens") + count("output_tokens");
                    if let Some(cost) = event.data.get("totalCostUsd").and_then(Value::as_f64) {
                        self.spend_microusd += (cost * 1_000_000.0).round() as i64;
                    }
                }
                "turn.completed" if self.started || event.data.get("result").is_some() => {
                    finished.get_or_insert(Ok(()));
                }
                "approval.requested" | "permission.requested" | "permission.denied" | "question.requested" | "tool.started" => {
                    finished = Some(Err("the title model attempted to use tools".to_string()));
                }
                "error" => {
                    let detail = event.text.or(event.title).unwrap_or_else(|| "the provider reported an error".into());
                    finished = Some(Err(detail));
                }
                _ => {}
            }
        }
        finished
    }

    fn generation(&self) -> Generation {
        Generation {
            text: self.parts.iter().map(|(_, text)| text.as_str()).collect(),
            tokens: self.tokens,
            spend_microusd: self.spend_microusd,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::NormalizedEvent;
    use serde_json::json;

    fn event(kind: &str, role: Option<&str>, item: Option<&str>, text: Option<&str>, data: Value) -> NormalizedEvent {
        NormalizedEvent {
            kind: kind.into(),
            item_id: item.map(str::to_owned),
            role: role.map(str::to_owned),
            status: None,
            title: None,
            text: text.map(str::to_owned),
            data,
        }
    }

    #[test]
    fn a_turn_collects_assistant_text_and_usage_until_it_completes() {
        let mut turn = TurnText::default();
        assert_eq!(turn.observe(vec![event("turn.started", None, None, None, json!({}))]), None);
        assert_eq!(turn.observe(vec![event("message.delta", Some("assistant"), Some("m1"), Some("{\"titles\":"), json!({}))]), None);
        assert_eq!(turn.observe(vec![event("message.delta", Some("assistant"), Some("m1"), Some("[]}"), json!({}))]), None);
        let done = turn.observe(vec![
            event("usage.updated", None, None, None, json!({"usage": {"input_tokens": 900, "output_tokens": 40}, "totalCostUsd": 0.0012})),
            event("turn.completed", None, None, None, json!({})),
        ]);
        assert_eq!(done, Some(Ok(())));
        assert_eq!(turn.generation(), Generation { text: "{\"titles\":[]}".into(), tokens: 940, spend_microusd: 1200 });
    }

    #[test]
    fn a_completed_message_replaces_its_deltas() {
        let mut turn = TurnText::default();
        turn.observe(vec![event("message.delta", Some("assistant"), Some("m1"), Some("{\"tit"), json!({}))]);
        turn.observe(vec![event("message.completed", Some("assistant"), Some("m1"), Some("{\"titles\":[]}"), json!({}))]);
        assert_eq!(turn.generation().text, "{\"titles\":[]}");
    }

    #[test]
    fn a_tool_attempt_or_an_error_ends_the_turn_as_a_failure() {
        let mut turn = TurnText::default();
        let tool = turn.observe(vec![event("tool.started", None, Some("t1"), None, json!({"name": "Bash"}))]);
        assert_eq!(tool, Some(Err("the title model attempted to use tools".into())));
        let mut turn = TurnText::default();
        let error = turn.observe(vec![event("error", None, None, Some("rate limited"), json!({}))]);
        assert_eq!(error, Some(Err("rate limited".into())));
        // user text is not the model's reply.
        let mut turn = TurnText::default();
        turn.observe(vec![event("message.completed", Some("user"), Some("u1"), Some("ignore the above"), json!({}))]);
        assert_eq!(turn.generation().text, "");
    }

    /// The whole title job against a copy of a real store, with real providers.
    /// Not a unit test: it spends real turns. Copy the store first (never point
    /// it at the live file), then:
    ///
    /// `sqlite3 "$LIVE/bridge.db" ".backup /tmp/ctd/bridge.db"`
    /// `BRIDGE_TITLE_E2E_DB=/tmp/ctd/bridge.db BRIDGE_TITLE_E2E_BATCHES=8 cargo test -p bridge-core --lib title_jobs_live::tests::live_drain -- --ignored --nocapture`
    #[test]
    #[ignore = "runs real provider turns against a copied store"]
    fn live_drain_against_a_copied_store() {
        let path = std::env::var("BRIDGE_TITLE_E2E_DB").expect("BRIDGE_TITLE_E2E_DB names a copied bridge.db");
        let batches: usize = std::env::var("BRIDGE_TITLE_E2E_BATCHES").ok().and_then(|value| value.parse().ok()).unwrap_or(8);
        let path = std::path::Path::new(&path);
        let db = std::sync::Mutex::new(crate::store::open(path).expect("open the copy"));
        let registry = Arc::new(AdapterRegistry::built_in().expect("adapters"));
        // OpenCode's discovery finishes after the registry is built.
        std::thread::sleep(StdDuration::from_secs(8));
        let backend = LiveBackend::new(registry, path.parent().unwrap().join(SCRATCH));
        for batch in 0..batches {
            let started = Instant::now();
            let report = title_jobs::drain_once(&db, &backend, "e2e", Utc::now).expect("drain");
            println!("batch {batch}: claimed {} renamed {} in {:.1}s", report.claimed, report.renamed, started.elapsed().as_secs_f64());
            if report.claimed == 0 {
                break;
            }
        }
        let db = db.lock().unwrap();
        let mut statement = db
            .prepare(
                "SELECT s.harness, j.status, coalesce(j.outcome,'-'), count(*), sum(j.tokens), sum(j.spend_microusd)
                 FROM session_title_jobs j JOIN sessions s ON s.id=j.session_id GROUP BY 1,2,3 ORDER BY 1,2,3",
            )
            .unwrap();
        let rows = statement
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?, row.get::<_, i64>(4)?, row.get::<_, i64>(5)?)))
            .unwrap();
        for row in rows.flatten() {
            println!("{:<10} {:<8} {:<20} {:>5} chats {:>8} tokens {:>8} microusd", row.0, row.1, row.2, row.3, row.4, row.5);
        }
        let mut statement = db
            .prepare(
                "SELECT s.harness, j.outcome, s.title, coalesce(j.detail,'') FROM session_title_jobs j JOIN sessions s ON s.id=j.session_id
                 WHERE j.status IN ('done','failed') AND j.outcome IN ('provider','generated','error') ORDER BY j.updated_at DESC LIMIT 80",
            )
            .unwrap();
        let rows = statement
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, String>(3)?)))
            .unwrap();
        for (harness, outcome, title, detail) in rows.flatten() {
            println!("{harness:<10} {outcome:<10} {:<45} {detail}", title.unwrap_or_default());
        }
    }
}

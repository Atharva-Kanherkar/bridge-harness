//! The composer's inline draft-completion typeahead.
//!
//! Settings are stored the same way Work's are (`configuration_entries`, a
//! `configured` flag that distinguishes "never set up" from "saved off") —
//! see [`crate::work`]. What is new here is the engine: a single warm hidden
//! session (`kind = 'suggestion'`, no workspace, so it never appears in a
//! session list) that the composer asks to continue whatever the user has
//! typed but not sent. The session is recycled after [`MAX_WARM_TURNS`] turns
//! so its own context does not grow without bound, and it is torn down and
//! restarted after any failed turn — a session that just errored is not
//! trusted to still be good for the next one.
//!
//! A request that fails with a signal this module recognises as
//! `unknown_model`, `unauthorized`, or `rate_limited` retries once against
//! [`FALLBACK_PROVIDER`]/[`FALLBACK_MODEL`], and the configured model/provider
//! pair earns a cooldown so a broken configuration is not re-probed on every
//! keystroke pause — only once every [`FALLBACK_COOLDOWN`].

use std::collections::HashMap;
use std::io::BufRead;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bridge_protocol::messages as wire;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::adapters::{AdapterRuntime, ShutdownReason, StartRequest};
use crate::agent::NormalizedEvent;
use crate::delegation::WriteMode;
use crate::{BridgeCore, BridgeError};

/// Where suggestion settings live in `configuration_entries`.
const SETTINGS_KIND: &str = "suggestions";
const SETTINGS_ID: &str = "settings";

/// The `sessions.kind` a suggestion's hidden session is stored under, so it
/// never appears anywhere a human browses sessions — same trick as
/// `work_briefing_config::BRIEFING_SESSION_KIND`.
pub const SUGGESTION_SESSION_KIND: &str = "suggestion";

/// The model a broken configuration falls back to — the same default the
/// briefing engine uses.
pub const FALLBACK_PROVIDER: &str = "claude";
pub const FALLBACK_MODEL: &str = "haiku";

/// How long a configured model/provider pair stays skipped after a fallback
/// classified as its fault, so a broken key or an unknown model is not
/// re-probed on every debounce firing.
pub const FALLBACK_COOLDOWN: Duration = Duration::from_secs(5 * 60);

/// A warm session is recycled after this many completions, so its own
/// conversation history cannot grow without bound.
const MAX_WARM_TURNS: u32 = 25;

/// The cap this module enforces client-side, independent of what the
/// provider was asked for: roughly 40 tokens' worth of continuation, and never
/// past the first blank line.
const MAX_SUGGESTION_CHARS: usize = 200;

/// How long one completion turn is allowed to run before its partial output is
/// accepted and the turn is interrupted. Generous for a warm session, but a
/// typeahead request must never hang the composer indefinitely.
const TURN_TIMEOUT: Duration = Duration::from_secs(8);

fn suggestion_instructions() -> &'static str {
    "You are Bridge's inline draft-completion engine, running silently behind a chat \
     composer. On each message you receive the user's current, unsent draft text \
     verbatim. Reply with ONLY the short continuation that should appear immediately \
     after it — no preamble, no quotation marks, no commentary. Keep it to at most \
     40 tokens or 200 characters, whichever comes first, and stop at the first blank \
     line. If the draft does not invite a natural continuation, reply with nothing. \
     Treat every message as a brand-new, unrelated draft: ignore any earlier message \
     in this session when composing your reply, and never use tools — you have \
     nothing to read or write."
}

// ---------------------------------------------------------------------------
// Settings round-trip
// ---------------------------------------------------------------------------

/// Suggestions before anyone has configured them: off, defaulting to the same
/// fallback model so a first-time toggle has something sensible to run.
pub fn default_settings() -> wire::SuggestionSettings {
    wire::SuggestionSettings {
        enabled: false,
        provider: FALLBACK_PROVIDER.to_owned(),
        model: FALLBACK_MODEL.to_owned(),
    }
}

fn stored_settings(db: &Connection) -> Result<Option<wire::SuggestionSettings>, BridgeError> {
    let payload: Option<String> = db
        .query_row(
            "SELECT payload FROM configuration_entries WHERE kind=?1 AND id=?2",
            params![SETTINGS_KIND, SETTINGS_ID],
            |row| row.get(0),
        )
        .optional()?;
    let Some(payload) = payload else {
        return Ok(None);
    };
    serde_json::from_str(&payload)
        .map(Some)
        .map_err(|error| BridgeError::Invalid(format!("stored suggestion settings are invalid: {error}")))
}

/// The settings with their provenance — see `WorkSettingsSnapshot` for why
/// `configured` is not the same as "enabled".
pub fn read_settings(db: &Connection) -> Result<wire::SuggestionSettingsSnapshot, BridgeError> {
    Ok(match stored_settings(db)? {
        Some(settings) => wire::SuggestionSettingsSnapshot { configured: true, settings },
        None => wire::SuggestionSettingsSnapshot { configured: false, settings: default_settings() },
    })
}

pub fn validate_settings(settings: &wire::SuggestionSettings) -> Result<(), String> {
    if settings.provider.trim().is_empty() {
        return Err("the suggestion provider cannot be blank".into());
    }
    if settings.model.trim().is_empty() {
        return Err("the suggestion model cannot be blank".into());
    }
    Ok(())
}

/// Persist suggestion settings. The only production writer.
pub fn write_settings(
    db: &Connection,
    settings: &wire::SuggestionSettings,
) -> Result<wire::SuggestionSettingsSnapshot, BridgeError> {
    validate_settings(settings).map_err(BridgeError::Invalid)?;
    let payload = serde_json::to_string(settings)
        .map_err(|error| BridgeError::Invalid(format!("settings could not be serialised: {error}")))?;
    let now = Utc::now().to_rfc3339();
    db.execute(
        "INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at)
         VALUES(?1,?2,?3,?4,?4)
         ON CONFLICT(kind,id) DO UPDATE SET payload=excluded.payload,updated_at=excluded.updated_at",
        params![SETTINGS_KIND, SETTINGS_ID, payload, now],
    )?;
    Ok(wire::SuggestionSettingsSnapshot { configured: true, settings: settings.clone() })
}

// ---------------------------------------------------------------------------
// Fallback classification and cooldown
// ---------------------------------------------------------------------------

/// What kind of trouble a completion attempt ran into, when it is the sort of
/// trouble a fallback can route around. `Serialize` only (never deserialized):
/// this is core's own classification, and `Serialize` exists solely so
/// `protocol_mirror` can drift-check it against `wire::SuggestionFallbackReason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackReason {
    UnknownModel,
    Unauthorized,
    RateLimited,
}

impl FallbackReason {
    pub fn as_wire(self) -> wire::SuggestionFallbackReason {
        match self {
            Self::UnknownModel => wire::SuggestionFallbackReason::UnknownModel,
            Self::Unauthorized => wire::SuggestionFallbackReason::Unauthorized,
            Self::RateLimited => wire::SuggestionFallbackReason::RateLimited,
        }
    }
}

/// Deliberately narrow, same discipline as `worker_retry::TRANSIENT_SIGNALS`:
/// a signal not on these lists is a plain failure, not a fallback trigger.
const UNAUTHORIZED_SIGNALS: &[&str] = &[
    "unauthorized",
    "authentication_error",
    "invalid api key",
    "invalid x-api-key",
    "invalid_api_key",
    "not authenticated",
    "not logged in",
    "401",
    "403",
    "forbidden",
];
const RATE_LIMITED_SIGNALS: &[&str] =
    &["rate_limit", "rate limit", "rate-limited", "429", "too many requests", "overloaded", "usage limit reached"];
const UNKNOWN_MODEL_SIGNALS: &[&str] =
    &["unknown model", "model not found", "no such model", "invalid model", "model_not_found"];

/// Classify a completion failure's text. Order matters: a 401/403 or a rate
/// limit is a more specific and more common signal than the generic wording
/// an "unknown model" refusal tends to use, so those are checked first.
pub fn classify_error(text: &str) -> Option<FallbackReason> {
    let lower = text.to_ascii_lowercase();
    if UNAUTHORIZED_SIGNALS.iter().any(|signal| lower.contains(signal)) {
        return Some(FallbackReason::Unauthorized);
    }
    if RATE_LIMITED_SIGNALS.iter().any(|signal| lower.contains(signal)) {
        return Some(FallbackReason::RateLimited);
    }
    if UNKNOWN_MODEL_SIGNALS.iter().any(|signal| lower.contains(signal)) {
        return Some(FallbackReason::UnknownModel);
    }
    None
}

/// Whether `started` is still within `cooldown` as of `now`. A pure function
/// of three instants so the 5-minute rule is testable without a real sleep.
pub fn in_cooldown(started: Instant, now: Instant, cooldown: Duration) -> bool {
    now.saturating_duration_since(started) < cooldown
}

// ---------------------------------------------------------------------------
// The engine
// ---------------------------------------------------------------------------

struct WarmSession {
    provider: String,
    model: String,
    session_id: String,
    runtime: Box<dyn AdapterRuntime>,
    receiver: mpsc::Receiver<String>,
    turns_used: u32,
}

#[derive(Default)]
struct EngineState {
    session: Option<WarmSession>,
    /// (provider, model) → when its cooldown started and why it was earned.
    cooldowns: HashMap<(String, String), (Instant, FallbackReason)>,
}

/// One warm hidden session, reused across completions and torn down on
/// failure or recycling. Lives on [`BridgeCore`] so it survives across
/// requests instead of paying process-start latency on every keystroke pause.
#[derive(Default)]
pub struct SuggestionEngine {
    state: Mutex<EngineState>,
}

impl SuggestionEngine {
    pub fn new() -> Self {
        Self::default()
    }

    fn active_cooldown(&self, provider: &str, model: &str) -> Option<FallbackReason> {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap();
        let key = (provider.to_owned(), model.to_owned());
        match state.cooldowns.get(&key).copied() {
            Some((started, reason)) if in_cooldown(started, now, FALLBACK_COOLDOWN) => Some(reason),
            Some(_) => {
                state.cooldowns.remove(&key);
                None
            }
            None => None,
        }
    }

    fn start_cooldown(&self, provider: &str, model: &str, reason: FallbackReason) {
        self.state
            .lock()
            .unwrap()
            .cooldowns
            .insert((provider.to_owned(), model.to_owned()), (Instant::now(), reason));
    }

    fn drop_session(core: &Arc<BridgeCore>, state: &mut EngineState, reason: ShutdownReason) {
        if let Some(mut session) = state.session.take() {
            let status = match reason {
                ShutdownReason::Failed => "failed",
                _ => "idle",
            };
            core.adapter_registry.forget_session(&session.provider, session.runtime.provider_session_id());
            session.runtime.stop(reason);
            settle_hidden_session(core, &session.session_id, status);
        }
    }

    /// Run one completion, reusing the warm session when it already matches
    /// `provider`/`model` and has turns left, otherwise recycling it.
    fn complete(
        &self,
        core: &Arc<BridgeCore>,
        provider: &str,
        model: &str,
        text: &str,
    ) -> Result<String, BridgeError> {
        let mut state = self.state.lock().unwrap();
        let needs_fresh = match &state.session {
            Some(session) => {
                session.provider != provider || session.model != model || session.turns_used >= MAX_WARM_TURNS
            }
            None => true,
        };
        if needs_fresh {
            Self::drop_session(core, &mut state, ShutdownReason::Completed);
            state.session = Some(start_warm_session(core, provider, model)?);
        }
        let outcome = {
            let session = state.session.as_mut().expect("a warm session was just ensured");
            run_turn(core, session, text)
        };
        match outcome {
            Ok(completion) => {
                if completion.interrupted {
                    // Late terminal frames from an interrupted turn cannot be
                    // reused as the next draft's completion.
                    Self::drop_session(core, &mut state, ShutdownReason::Completed);
                } else if let Some(session) = state.session.as_mut() {
                    session.turns_used += 1;
                }
                Ok(completion.text)
            }
            Err(error) => {
                // A session that just failed a turn is not trusted for the next
                // one — drop it so the following request starts clean.
                Self::drop_session(core, &mut state, ShutdownReason::Failed);
                Err(error)
            }
        }
    }
}

fn start_warm_session(core: &Arc<BridgeCore>, provider: &str, model: &str) -> Result<WarmSession, BridgeError> {
    let session_id = Uuid::new_v4().to_string();
    let scratch = core.chat_scratch_dir(&session_id);
    std::fs::create_dir_all(&scratch)?;
    let cwd = scratch.to_string_lossy().to_string();
    {
        let db = core.db.lock().unwrap();
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,model,kind,title,cwd,depth)
             VALUES(?1,NULL,?2,'Suggestions','working','reported',?3,?4,'Inline suggestions',?5,0)",
            params![session_id, provider, model, SUGGESTION_SESSION_KIND, cwd],
        )?;
    }
    let started = core.adapter_registry.start_completion(
        provider,
        StartRequest {
            cwd: &cwd,
            model: Some(model),
            effort: None,
            instructions: Some(suggestion_instructions()),
            // No file or shell access: this session only ever predicts text.
            write_mode: Some(WriteMode::ReadOnly),
            read_only_sandbox: None,
            briefing: None,
            on_progress: None,
        },
    );
    let started = match started {
        Ok(started) => started,
        Err(error) => {
            settle_hidden_session(core, &session_id, "failed");
            return Err(error);
        }
    };
    for message in &started.startup_messages {
        core.adapter_registry.normalize(provider, message);
    }
    let runtime = started.runtime;
    {
        let db = core.db.lock().unwrap();
        db.execute(
            "UPDATE sessions SET provider_session_id=?2,started_at=?3 WHERE id=?1",
            params![session_id, runtime.provider_session_id(), Utc::now().to_rfc3339()],
        )?;
    }
    let (sender, receiver) = mpsc::channel::<String>();
    let mut reader = started.reader;
    std::thread::spawn(move || {
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if sender.send(line.trim_end().to_owned()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    Ok(WarmSession {
        provider: provider.to_owned(),
        model: model.to_owned(),
        session_id,
        runtime,
        receiver,
        turns_used: 0,
    })
}

fn settle_hidden_session(core: &Arc<BridgeCore>, session_id: &str, status: &str) {
    let db = core.db.lock().unwrap();
    let _ = db.execute(
        "UPDATE sessions SET status=?2,ended_at=?3 WHERE id=?1",
        params![session_id, status, Utc::now().to_rfc3339()],
    );
}

/// One provider turn's accumulated text, read to completion, to the char cap,
/// or to the deadline — whichever comes first.
struct TurnBuffer {
    text: String,
    parts: Vec<(Option<String>, String)>,
    started: bool,
    done: bool,
    error: Option<String>,
}

impl TurnBuffer {
    fn new() -> Self {
        Self { text: String::new(), parts: Vec::new(), started: false, done: false, error: None }
    }

    fn observe(&mut self, events: Vec<NormalizedEvent>) {
        // Consume the whole batch: a failed turn can contain its terminal
        // event followed by the error that explains it.
        for event in events {
            match event.kind.as_str() {
                "turn.started" => self.started = true,
                "message.delta" | "message.completed" if event.role.as_deref() == Some("assistant") => {
                    self.started = true;
                    let Some(text) = event.text else { continue; };
                    if let Some((_, part)) = self.parts.iter_mut().find(|(id, _)| *id == event.item_id) {
                        if event.kind == "message.completed" { *part = text; } else { part.push_str(&text); }
                    } else {
                        self.parts.push((event.item_id, text));
                    }
                    self.text = self.parts.iter().map(|(_, text)| text.as_str()).collect();
                }
                "turn.completed" if self.started || event.data.get("result").is_some() => self.done = true,
                "approval.requested" | "permission.denied" | "tool.started" => {
                    self.error = Some("the suggestion model attempted to use tools".into());
                    self.done = true;
                }
                "error" => {
                    self.error = Some(event.text.or(event.title).unwrap_or_else(|| "the suggestion provider reported an error".into()));
                    self.done = true;
                }
                _ => {}
            }
        }
    }
}

struct TurnCompletion {
    text: String,
    interrupted: bool,
}

/// Whether accumulated text has already hit this module's own cap,
/// independent of whatever the provider itself was asked for.
fn capped(text: &str) -> bool {
    text.chars().count() >= MAX_SUGGESTION_CHARS || text.contains("\n\n")
}

/// Truncate to the first blank line, then to the char cap, then trim trailing
/// whitespace a mid-cap cut can leave behind.
fn trim_suggestion(text: &str) -> String {
    let stopped = match text.find("\n\n") {
        Some(index) => &text[..index],
        None => text,
    };
    let bounded: String = stopped.chars().take(MAX_SUGGESTION_CHARS).collect();
    bounded.trim_end().to_owned()
}

/// A typeahead cannot wait through a provider's scheduled account-limit retry.
/// OpenCode can emit retries indefinitely without a terminal error frame.
fn completion_retry_error(message: &Value) -> Option<String> {
    if message.get("type").and_then(Value::as_str) != Some("session.status") { return None; }
    let status = message.pointer("/properties/status")?;
    if status.get("type").and_then(Value::as_str) != Some("retry") { return None; }
    let detail = status.get("message").and_then(Value::as_str).unwrap_or("the suggestion provider is retrying");
    let reason = status.pointer("/action/reason").and_then(Value::as_str);
    Some(match reason { Some(reason) => format!("{reason}: {detail}"), None => detail.to_owned() })
}

fn run_turn(core: &Arc<BridgeCore>, session: &mut WarmSession, text: &str) -> Result<TurnCompletion, BridgeError> {
    session
        .runtime
        .send_turn(text)
        .map_err(|error| BridgeError::Invalid(format!("the suggestion turn could not be sent: {error}")))?;
    let deadline = Instant::now() + TURN_TIMEOUT;
    let mut buffer = TurnBuffer::new();
    let mut interrupted = false;
    loop {
        if capped(&buffer.text) {
            let _ = session.runtime.interrupt();
            interrupted = true;
            break;
        }
        if Instant::now() >= deadline {
            let _ = session.runtime.interrupt();
            interrupted = true;
            break;
        }
        match session.receiver.recv_timeout(Duration::from_millis(250)) {
            Ok(line) => {
                if let Ok(message) = serde_json::from_str::<Value>(&line) {
                    if let Some(detail) = completion_retry_error(&message) {
                        buffer.error = Some(detail);
                        buffer.done = true;
                    } else {
                        buffer.observe(core.adapter_registry.normalize(&session.provider, &message));
                    }
                }
                if buffer.done {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let detail = session
                    .runtime
                    .failure_context()
                    .unwrap_or_else(|| "the suggestion provider process ended".to_owned());
                return Err(BridgeError::Adapter(detail));
            }
        }
    }
    if let Some(detail) = buffer.error {
        return Err(BridgeError::Adapter(detail));
    }
    if interrupted && buffer.text.is_empty() {
        return Err(BridgeError::Adapter("the suggestion model did not respond within 8 seconds".into()));
    }
    Ok(TurnCompletion { text: trim_suggestion(&buffer.text), interrupted })
}

/// `models/suggest_completion`'s body. Disabled settings and an empty draft
/// both refuse to spend a model call — the empty-draft case returns an empty
/// suggestion rather than an error, since it is not a mistake, just nothing
/// to complete yet.
pub fn suggest_completion(
    core: &Arc<BridgeCore>,
    text: &str,
) -> Result<wire::SuggestCompletionResult, BridgeError> {
    let snapshot = read_settings(&core.db.lock().unwrap())?;
    if !snapshot.settings.enabled {
        return Err(BridgeError::Invalid("suggestions are turned off".into()));
    }
    if text.trim().is_empty() {
        return Ok(wire::SuggestCompletionResult { suggestion: String::new(), used_fallback: false, fallback_reason: None });
    }
    let provider = snapshot.settings.provider.as_str();
    let model = snapshot.settings.model.as_str();
    let engine = &core.suggestion_engine;

    if let Some(reason) = engine.active_cooldown(provider, model) {
        let suggestion = engine.complete(core, FALLBACK_PROVIDER, FALLBACK_MODEL, text)?;
        return Ok(wire::SuggestCompletionResult {
            suggestion,
            used_fallback: true,
            fallback_reason: Some(reason.as_wire()),
        });
    }

    match engine.complete(core, provider, model, text) {
        Ok(suggestion) => {
            Ok(wire::SuggestCompletionResult { suggestion, used_fallback: false, fallback_reason: None })
        }
        Err(error) => {
            let Some(reason) = classify_error(&error.to_string()) else {
                return Err(error);
            };
            if provider == FALLBACK_PROVIDER && model == FALLBACK_MODEL {
                // Already the fallback target: there is nowhere left to fall back to.
                return Err(error);
            }
            engine.start_cooldown(provider, model, reason);
            let suggestion = engine.complete(core, FALLBACK_PROVIDER, FALLBACK_MODEL, text)?;
            Ok(wire::SuggestCompletionResult {
                suggestion,
                used_fallback: true,
                fallback_reason: Some(reason.as_wire()),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_are_off_with_a_usable_fallback_model() {
        let settings = default_settings();
        assert!(!settings.enabled);
        assert_eq!(settings.provider, "claude");
        assert_eq!(settings.model, "haiku");
    }

    #[test]
    fn classification_prefers_the_most_specific_signal() {
        assert_eq!(classify_error("401 Unauthorized: invalid api key"), Some(FallbackReason::Unauthorized));
        assert_eq!(classify_error("Error: authentication_error — please check credentials"), Some(FallbackReason::Unauthorized));
        assert_eq!(classify_error("429 Too Many Requests, rate_limit_error"), Some(FallbackReason::RateLimited));
        assert_eq!(classify_error("Overloaded, please retry later"), Some(FallbackReason::RateLimited));
        assert_eq!(classify_error("model_not_found: no such model 'nonexistent'"), Some(FallbackReason::UnknownModel));
        assert_eq!(classify_error("the connection was reset by the peer"), None);
    }

    #[test]
    fn a_signal_that_matches_nothing_never_triggers_a_fallback() {
        assert_eq!(classify_error("the build failed"), None);
        assert_eq!(classify_error(""), None);
    }

    #[test]
    fn cooldown_covers_exactly_five_minutes_from_when_it_started() {
        let started = Instant::now();
        let just_under = started + Duration::from_secs(299);
        let exactly_at = started + FALLBACK_COOLDOWN;
        let just_over = started + Duration::from_secs(301);
        assert!(in_cooldown(started, started, FALLBACK_COOLDOWN), "the instant it starts");
        assert!(in_cooldown(started, just_under, FALLBACK_COOLDOWN));
        assert!(!in_cooldown(started, exactly_at, FALLBACK_COOLDOWN), "the boundary itself has elapsed");
        assert!(!in_cooldown(started, just_over, FALLBACK_COOLDOWN));
    }

    #[test]
    fn a_repeated_probe_within_the_cooldown_does_not_reset_its_clock() {
        // Simulates two calls to `active_cooldown` against the same started
        // instant: the second probe, later, must see less time remaining, not
        // a clock that restarted because someone asked.
        let started = Instant::now();
        let first_probe = started + Duration::from_secs(10);
        let second_probe = started + Duration::from_secs(250);
        assert!(in_cooldown(started, first_probe, FALLBACK_COOLDOWN));
        assert!(in_cooldown(started, second_probe, FALLBACK_COOLDOWN));
        assert!(!in_cooldown(started, second_probe + Duration::from_secs(60), FALLBACK_COOLDOWN));
    }

    #[test]
    fn suggestions_stop_at_the_first_blank_line() {
        assert_eq!(trim_suggestion("finish this thought\n\nand then some unrelated text"), "finish this thought");
    }

    #[test]
    fn suggestions_are_capped_at_two_hundred_characters() {
        let long = "a".repeat(500);
        let trimmed = trim_suggestion(&long);
        assert_eq!(trimmed.chars().count(), MAX_SUGGESTION_CHARS);
    }

    #[test]
    fn trailing_whitespace_left_by_a_mid_cap_cut_is_trimmed() {
        let text = format!("{}   ", "a".repeat(50));
        assert_eq!(trim_suggestion(&text), "a".repeat(50));
    }

    #[test]
    fn the_capped_predicate_matches_the_char_limit_and_the_blank_line_stop() {
        assert!(!capped("short"));
        assert!(capped(&"a".repeat(MAX_SUGGESTION_CHARS)));
        assert!(capped("stop\n\nhere"));
    }

    #[test]
    fn opencode_completion_is_read_from_actual_wire_frames() {
        let mut buffer = TurnBuffer::new();
        let mut state = crate::agent::OpenCodeStreamState::default();
        buffer.observe(crate::agent::normalize_opencode_message_with_state(&serde_json::json!({"type":"message.updated", "properties":{"sessionID":"s", "info":{"id":"m", "role":"assistant"}}}), &mut state));
        buffer.observe(crate::agent::normalize_opencode_message_with_state(&serde_json::json!({
            "type":"message.part.updated", "properties":{"sessionID":"s", "part":{
                "id":"p", "messageID":"m", "type":"text", "text":" release notes", "time":{"start":1,"end":2}
            }}
        }), &mut state));
        assert_eq!(buffer.text, " release notes");
    }

    #[test]
    fn codex_deltas_are_replaced_by_final_text_without_duplicates() {
        let mut state = crate::agent::CodexStreamState::default();
        let mut buffer = TurnBuffer::new();
        for frame in [
            serde_json::json!({"method":"turn/started", "params":{"turn":{"id":"t"}}}),
            serde_json::json!({"method":"item/reasoning/textDelta", "params":{"itemId":"r", "delta":"secret thought"}}),
            serde_json::json!({"method":"item/agentMessage/delta", "params":{"itemId":"m", "delta":" release"}}),
            serde_json::json!({"method":"item/completed", "params":{"item":{"id":"m", "type":"agentMessage", "text":" release notes"}}}),
            serde_json::json!({"method":"turn/completed", "params":{"turn":{"status":"completed"}}}),
        ] { buffer.observe(crate::agent::normalize_codex_message_with_state(&frame, &mut state)); }
        assert_eq!(buffer.text, " release notes");
        assert!(buffer.done);
        assert!(buffer.error.is_none());
    }

    #[test]
    fn opencode_deltas_and_snapshots_are_deduplicated_and_idle_finishes_the_turn() {
        let mut state = crate::agent::OpenCodeStreamState::default();
        let mut buffer = TurnBuffer::new();
        for frame in [
            serde_json::json!({"type":"session.status", "properties":{"sessionID":"s", "status":{"type":"idle"}}}),
            serde_json::json!({"type":"session.status", "properties":{"sessionID":"s", "status":{"type":"busy"}}}),
            serde_json::json!({"type":"message.updated", "properties":{"sessionID":"s", "info":{"id":"m", "role":"assistant"}}}),
            serde_json::json!({"type":"message.part.updated", "properties":{"sessionID":"s", "part":{"id":"p", "messageID":"m", "type":"text", "text":"", "time":{"start":1}}}}),
            serde_json::json!({"type":"message.part.delta", "properties":{"sessionID":"s", "partID":"p", "messageID":"m", "field":"text", "delta":" release"}}),
            serde_json::json!({"type":"message.part.updated", "properties":{"sessionID":"s", "part":{"id":"p", "messageID":"m", "type":"text", "text":" release notes", "time":{"start":1,"end":2}}}}),
            serde_json::json!({"type":"session.status", "properties":{"sessionID":"s", "status":{"type":"idle"}}}),
        ] { buffer.observe(crate::agent::normalize_opencode_message_with_state(&frame, &mut state)); }
        assert_eq!(buffer.text, " release notes");
        assert!(buffer.done);
    }

    #[test]
    fn claude_completion_preserves_leading_space_and_ignores_user_and_thinking() {
        let mut state = crate::agent::ClaudeStreamState::default();
        let mut buffer = TurnBuffer::new();
        for frame in [
            serde_json::json!({"type":"user", "message":{"id":"u", "role":"user", "content":[{"type":"text", "text":"draft"}]}}),
            serde_json::json!({"type":"assistant", "message":{"id":"a", "role":"assistant", "content":[{"type":"thinking", "thinking":"thoughts"},{"type":"text", "text":" release notes"}]}}),
            serde_json::json!({"type":"result", "subtype":"success", "is_error":false, "result":" release notes"}),
        ] { buffer.observe(crate::agent::normalize_claude_message_with_state(&frame, &mut state)); }
        assert_eq!(buffer.text, " release notes");
        assert!(buffer.done);
    }

    #[test]
    fn failed_terminal_batches_keep_the_provider_error() {
        let mut buffer = TurnBuffer::new();
        let events = crate::agent::normalize_codex_message(&serde_json::json!({
            "method":"turn/completed", "params":{"turn":{"status":"failed", "error":{"message":"rate_limited"}}}
        }));
        buffer.observe(events);
        assert!(buffer.done);
        assert_eq!(buffer.error.as_deref(), Some("rate_limited"));
        let mut claude = TurnBuffer::new();
        claude.observe(crate::agent::normalize_claude_message(&serde_json::json!({"type":"result", "subtype":"error", "is_error":true, "result":"unauthorized"})));
        assert_eq!(claude.error.as_deref(), Some("unauthorized"));
        let mut opencode = TurnBuffer::new();
        opencode.observe(crate::agent::normalize_opencode_message_with_state(&serde_json::json!({"type":"session.error", "properties":{"sessionID":"s", "error":{"name":"APIError", "data":{"message":"unknown model"}}}}), &mut crate::agent::OpenCodeStreamState::default()));
        assert!(opencode.error.is_some());
    }

    struct CompletionRuntime {
        interrupted: Arc<std::sync::atomic::AtomicUsize>,
        stopped: Arc<std::sync::atomic::AtomicUsize>,
    }
    impl AdapterRuntime for CompletionRuntime {
        fn process_id(&self) -> u32 { 0 }
        fn provider_session_id(&self) -> &str { "completion-test" }
        fn current_turn(&self) -> Arc<Mutex<Option<String>>> { Arc::new(Mutex::new(None)) }
        fn send_turn(&self, _: &str) -> Result<(), BridgeError> { Ok(()) }
        fn interrupt(&self) -> Result<(), BridgeError> { self.interrupted.fetch_add(1, std::sync::atomic::Ordering::SeqCst); Ok(()) }
        fn respond(&self, _: Value, _: &str) -> Result<(), BridgeError> { Ok(()) }
        fn stop(&mut self, _: ShutdownReason) { self.stopped.fetch_add(1, std::sync::atomic::Ordering::SeqCst); }
    }
    struct CompletionAdapter {
        starts: Arc<std::sync::atomic::AtomicUsize>,
        interrupted: Arc<std::sync::atomic::AtomicUsize>,
        stopped: Arc<std::sync::atomic::AtomicUsize>,
    }
    impl crate::adapters::HarnessAdapter for CompletionAdapter {
        fn as_any(&self) -> &dyn std::any::Any { self }
        fn descriptor(&self) -> crate::model::AdapterDescriptor {
            crate::model::AdapterDescriptor {
                id: "completion-test".into(), label: "Completion test".into(), available: true,
                auth_state: crate::model::AuthState::Unknown, version: Some("test".into()),
                capabilities: vec!["messages".into()], sandbox_modes: vec![], unavailable_reason: None,
                models: vec![], default_model: None, model_catalog: crate::model::ModelCatalogDiagnostics::curated(),
            }
        }
        fn start(&self, _: StartRequest<'_>) -> Result<crate::adapters::StartedAdapter, BridgeError> {
            let turn = self.starts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let text = if turn == 0 { "a".repeat(MAX_SUGGESTION_CHARS + 1) } else { " fresh".into() };
            let frame = serde_json::json!({"type":"assistant", "message":{"id":"m", "role":"assistant", "content":[{"type":"text", "text":text}]}});
            let result = serde_json::json!({"type":"result", "subtype":"success", "result":text});
            Ok(crate::adapters::StartedAdapter {
                runtime: Box::new(CompletionRuntime { interrupted: self.interrupted.clone(), stopped: self.stopped.clone() }),
                reader: Box::new(std::io::Cursor::new(format!("{frame}\n{result}\n"))), startup_messages: vec![],
            })
        }
        fn resume(&self, _: crate::adapters::ResumeRequest<'_>) -> Result<crate::adapters::StartedAdapter, BridgeError> { unreachable!() }
        fn supports_native_resume(&self) -> bool { false }
        fn normalize(&self, frame: &Value) -> Vec<NormalizedEvent> { crate::agent::normalize_claude_message(frame) }
    }

    #[test]
    fn capped_turn_recycles_the_runtime_before_the_next_draft() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let scratch = tempfile::tempdir().unwrap();
        let mut core = BridgeCore::for_tests(scratch.path());
        let starts = Arc::new(AtomicUsize::new(0));
        let interrupted = Arc::new(AtomicUsize::new(0));
        let stopped = Arc::new(AtomicUsize::new(0));
        let mut registry = crate::adapters::AdapterRegistry::empty();
        registry.register(Box::new(CompletionAdapter { starts: starts.clone(), interrupted: interrupted.clone(), stopped: stopped.clone() })).unwrap();
        core.adapter_registry = Arc::new(registry);
        let core = Arc::new(core);
        let engine = SuggestionEngine::new();
        assert_eq!(engine.complete(&core, "completion-test", "test", "first").unwrap(), "a".repeat(MAX_SUGGESTION_CHARS));
        assert!(engine.state.lock().unwrap().session.is_none());
        assert_eq!(interrupted.load(Ordering::SeqCst), 1);
        assert_eq!(stopped.load(Ordering::SeqCst), 1);
        assert_eq!(engine.complete(&core, "completion-test", "test", "second").unwrap(), " fresh");
        assert_eq!(starts.load(Ordering::SeqCst), 2);
        assert_eq!(engine.state.lock().unwrap().session.as_ref().unwrap().turns_used, 1);
    }

    #[test]
    #[ignore = "requires an authenticated OpenCode model; set BRIDGE_SUGGESTION_LIVE_MODEL"]
    fn live_inline_opencode_completion_uses_the_tool_denied_session() {
        let model = std::env::var("BRIDGE_SUGGESTION_LIVE_MODEL").expect("set BRIDGE_SUGGESTION_LIVE_MODEL");
        let scratch = tempfile::tempdir().unwrap();
        let mut core = BridgeCore::for_tests(scratch.path());
        core.adapter_registry = Arc::new(crate::adapters::AdapterRegistry::built_in().unwrap());
        let core = Arc::new(core);
        let deadline = Instant::now() + Duration::from_secs(30);
        while !core.adapter_registry.descriptor("opencode").is_some_and(|adapter| adapter.available) {
            assert!(Instant::now() < deadline, "OpenCode discovery did not finish");
            std::thread::sleep(Duration::from_millis(100));
        }
        write_settings(&core.db.lock().unwrap(), &wire::SuggestionSettings { enabled: true, provider: "opencode".into(), model }).unwrap();
        let result = suggest_completion(&core, "Please write a brief release note explaining the composer fixes, including").unwrap();
        SuggestionEngine::drop_session(&core, &mut core.suggestion_engine.state.lock().unwrap(), ShutdownReason::Completed);
        if std::env::var_os("BRIDGE_SUGGESTION_EXPECT_FALLBACK").is_some() {
            assert!(result.used_fallback, "the exhausted OpenCode account should use the fallback");
            assert_eq!(result.fallback_reason, Some(wire::SuggestionFallbackReason::RateLimited));
        } else {
            assert!(!result.used_fallback, "the configured OpenCode model should serve the completion");
        }
        assert!(!result.suggestion.trim().is_empty(), "the provider returned no draft continuation");
        assert!(result.suggestion.chars().count() <= MAX_SUGGESTION_CHARS);
        println!("Live completion returned {} characters; fallback={}", result.suggestion.chars().count(), result.used_fallback);
    }

    #[test]
    fn opencode_account_limit_retry_triggers_the_rate_limited_fallback() {
        let message = serde_json::json!({"type":"session.status", "properties":{"sessionID":"s", "status":{
            "type":"retry", "message":"monthly usage limit reached", "action":{"reason":"account_rate_limit"}, "next":9999999999999_u64
        }}});
        let error = completion_retry_error(&message).unwrap();
        assert_eq!(classify_error(&error), Some(FallbackReason::RateLimited));
        assert_eq!(classify_error("monthly usage limit reached"), Some(FallbackReason::RateLimited));
        assert!(completion_retry_error(&serde_json::json!({"type":"session.status", "properties":{"status":{"type":"busy"}}})).is_none());
    }

    #[test]
    fn the_instructions_forbid_tool_use_and_cross_draft_memory() {
        let instructions = suggestion_instructions();
        assert!(instructions.contains("never use tools"));
        assert!(instructions.contains("brand-new, unrelated draft"));
    }
}

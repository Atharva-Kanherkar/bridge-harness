//! The hidden reviewer session: one bounded, headless turn on Claude, Codex
//! or OpenCode that reads the prepared PR checkout and answers with the
//! findings contract.
//!
//! The session is offline and read-only by construction. It runs in the PR
//! checkout inside the read-only sandbox with no network, with each
//! harness's read-only file tools (Claude: Read, Grep, Glob; Codex: its
//! read-only sandbox; OpenCode: read, grep, glob and list, everything else
//! denied). Bridge, not the reviewer, touches GitHub. Every permission request
//! or question is declined on the spot and counted, so nothing waits for a
//! person, and the session has a wall clock, a tool-call cap and a cancel.
//!
//! A review never resumes an earlier session: an old transcript describes
//! code at an old head, and the checkout it read is gone. No `sessions` row is
//! written; the pass record holds the usage.
//!
//! The instructions are the stable prefix (role, offline rules, repository
//! conventions, review rules, output contract) and the turn carries what
//! varies (the PR, the context paths, what the diff left out), so provider
//! prompt caches hit across the session's tool turns and across passes.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::contract::{self, RawReview, OUTPUT_CONTRACT};
use super::steps::{CancelToken, PassOutcome, PassRequest, StepError};
use crate::adapters::{AdapterRegistry, AdapterRuntime, ShutdownReason, StartRequest};
use crate::agent::NormalizedEvent;
use crate::delegation::WriteMode;
use crate::runtime::BridgeCore;
use crate::worker_sandbox::ReadOnlySandbox;

/// Where the context folder is copied inside the checkout, so every harness
/// reads it with the same relative paths and none needs a directory outside
/// its working directory.
pub const CONTEXT_DIR: &str = ".bridge-review-context";

/// What a review looks for when the person wrote no instructions of their own.
pub const REVIEW_INSTRUCTIONS: &str = "Review the change for correctness bugs, security problems, risky \
changes, missing or broken tests, and clear maintainability problems. Read the surrounding code \
before judging a change. Cite concrete files and lines. Prefer a few well-founded findings to many \
speculative ones, and say nothing about formatting a formatter would fix.";

/// How long and how far one session may go.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_wall: Duration,
    pub max_tool_calls: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_wall: Duration::from_secs(900), max_tool_calls: 300 }
    }
}

/// What a session starts with.
#[derive(Debug, Clone, Copy)]
pub struct SessionStart<'a> {
    pub cwd: &'a Path,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub instructions: &'a str,
}

/// What the next read from a session found.
pub enum Next {
    Events(Vec<NormalizedEvent>),
    /// Nothing yet.
    Idle,
    /// The provider process ended.
    Ended,
}

/// One running provider session, as the reviewer loop drives it.
pub trait Session: Send {
    fn send_turn(&mut self, text: &str) -> Result<(), String>;
    fn next(&mut self, wait: Duration) -> Next;
    fn decline(&mut self, request_id: Value) -> Result<(), String>;
    fn reject_question(&mut self, request_id: Value) -> Result<(), String>;
    fn stop(&mut self, completed: bool);
}

/// Starts reviewer sessions. The registry in production; scripts in tests.
pub trait Transport: Send + Sync {
    fn start(&self, harness: &str, start: &SessionStart<'_>) -> Result<Box<dyn Session>, String>;
}

/// The real harnesses, each started as a reviewer inside an offline
/// read-only sandbox rooted at the checkout.
pub struct RegistryTransport {
    pub registry: Arc<AdapterRegistry>,
}

struct RegistrySession {
    harness: String,
    registry: Arc<AdapterRegistry>,
    runtime: Box<dyn AdapterRuntime>,
    lines: Receiver<String>,
    sandbox: ReadOnlySandbox,
}

impl Transport for RegistryTransport {
    fn start(&self, harness: &str, start: &SessionStart<'_>) -> Result<Box<dyn Session>, String> {
        let sandbox = ReadOnlySandbox::offline(start.cwd).map_err(|error| error.to_string())?;
        let cwd = start.cwd.to_string_lossy().into_owned();
        let started = self.registry.start_reviewer(
            harness,
            StartRequest {
                session_id: None,
                cwd: &cwd,
                model: start.model,
                effort: start.effort,
                speed: None,
                instructions: Some(start.instructions),
                write_mode: Some(WriteMode::ReadOnly),
                read_only_sandbox: Some(&sandbox),
                briefing: None,
                interactive: false,
                account: None,
                on_progress: None,
            },
        );
        let started = match started {
            Ok(started) => started,
            Err(error) => {
                sandbox.cleanup();
                return Err(error.to_string());
            }
        };
        let (sender, lines) = mpsc::channel::<String>();
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
        Ok(Box::new(RegistrySession {
            harness: harness.to_owned(),
            registry: self.registry.clone(),
            runtime: started.runtime,
            lines,
            sandbox,
        }))
    }
}

impl Session for RegistrySession {
    fn send_turn(&mut self, text: &str) -> Result<(), String> {
        self.runtime.send_turn(text).map_err(|error| error.to_string())
    }

    fn next(&mut self, wait: Duration) -> Next {
        match self.lines.recv_timeout(wait) {
            Ok(line) => match serde_json::from_str::<Value>(&line) {
                Ok(value) => Next::Events(self.registry.normalize(&self.harness, &value)),
                Err(_) => Next::Idle,
            },
            Err(RecvTimeoutError::Timeout) => Next::Idle,
            Err(RecvTimeoutError::Disconnected) => Next::Ended,
        }
    }

    fn decline(&mut self, request_id: Value) -> Result<(), String> {
        // "decline" is the one decision every adapter accepts as a refusal.
        self.runtime.respond(request_id, "decline").map_err(|error| error.to_string())
    }

    fn reject_question(&mut self, request_id: Value) -> Result<(), String> {
        self.runtime.reject_question(request_id.clone()).or_else(|_| self.runtime.respond(request_id, "decline")).map_err(|error| error.to_string())
    }

    fn stop(&mut self, completed: bool) {
        self.runtime.stop(if completed { ShutdownReason::Completed } else { ShutdownReason::Failed });
        self.sandbox.cleanup();
    }
}

/// What a session cost and did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Usage {
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_microusd: u64,
    pub denied_requests: u32,
    pub tool_calls: usize,
    /// The tool names the session used, in order: what a live check reads.
    pub tools: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionError {
    Start(String),
    Provider(String),
    Ended,
    Deadline(Duration),
    ToolLimit(usize),
    Contract(String),
    Cancelled,
}

impl SessionError {
    /// What the runner should make of it: try again, try another harness, or
    /// give up.
    pub fn into_step(self) -> StepError {
        match self {
            SessionError::Cancelled => StepError::Cancelled,
            SessionError::Start(message) | SessionError::Provider(message) if is_transient(&message) => {
                StepError::Transient(message)
            }
            SessionError::Start(message) => StepError::Harness(format!("The reviewer could not start: {message}")),
            SessionError::Provider(message) => StepError::Harness(message),
            SessionError::Ended => StepError::Harness("The reviewer stopped before it finished".into()),
            SessionError::Deadline(limit) => {
                StepError::Harness(format!("The reviewer ran past its {}-minute limit", limit.as_secs().div_ceil(60)))
            }
            SessionError::ToolLimit(limit) => StepError::Fatal(format!("The reviewer used more than {limit} tools")),
            SessionError::Contract(message) => {
                StepError::Fatal(format!("The reviewer's answer did not follow the findings contract: {message}"))
            }
        }
    }
}

/// Worth another try: the provider is busy, not broken.
fn is_transient(message: &str) -> bool {
    let lower = message.to_lowercase();
    ["rate limit", "rate_limit", "429", "overloaded", "temporarily", "503", "502", "timed out", "timeout", "connection reset"]
        .iter()
        .any(|needle| lower.contains(needle))
}

fn request_id(event: &NormalizedEvent) -> Value {
    ["requestId", "request_id", "id"]
        .iter()
        .find_map(|key| event.data.get(key).cloned())
        .unwrap_or(Value::Null)
}

/// Drive one turn to its end. Returns the final assistant message.
fn turn(
    session: &mut dyn Session,
    text: &str,
    usage: &mut Usage,
    deadline: Instant,
    limits: Limits,
    cancel: &CancelToken,
) -> Result<String, SessionError> {
    session.send_turn(text).map_err(SessionError::Provider)?;
    let mut parts: Vec<(Option<String>, String)> = Vec::new();
    let mut started = false;
    let mut result_text: Option<String> = None;
    loop {
        if cancel.is_cancelled() {
            return Err(SessionError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(SessionError::Deadline(limits.max_wall));
        }
        let events = match session.next(Duration::from_millis(200)) {
            Next::Idle => continue,
            Next::Ended => return Err(SessionError::Ended),
            Next::Events(events) => events,
        };
        let mut done = false;
        for event in events {
            match event.kind.as_str() {
                "turn.started" => started = true,
                "message.delta" | "message.completed" if event.role.as_deref() == Some("assistant") => {
                    started = true;
                    let Some(text) = event.text else { continue };
                    match parts.iter_mut().find(|(id, _)| *id == event.item_id) {
                        Some((_, part)) if event.kind == "message.completed" => *part = text,
                        Some((_, part)) => part.push_str(&text),
                        None => parts.push((event.item_id, text)),
                    }
                }
                "usage.updated" => {
                    let count = |key: &str| event.data.pointer(&format!("/usage/{key}")).and_then(Value::as_u64).unwrap_or(0);
                    usage.tokens_in += count("input_tokens");
                    usage.tokens_out += count("output_tokens");
                    let cost = event.data.get("totalCostUsd").or_else(|| event.data.get("cost")).and_then(Value::as_f64);
                    if let Some(cost) = cost.filter(|cost| cost.is_finite() && *cost > 0.0) {
                        usage.cost_microusd += (cost * 1_000_000.0).round() as u64;
                    }
                }
                "tool.started" => {
                    started = true;
                    usage.tool_calls += 1;
                    usage.tools.push(event.title.clone().unwrap_or_default());
                    if usage.tool_calls > limits.max_tool_calls {
                        return Err(SessionError::ToolLimit(limits.max_tool_calls));
                    }
                }
                // Nobody answers a hidden session: refuse at once, and count.
                "permission.requested" | "approval.requested" => {
                    usage.denied_requests += 1;
                    if let Err(error) = session.decline(request_id(&event)) {
                        eprintln!("[reviews] could not decline a permission request: {error}");
                    }
                }
                "question.requested" => {
                    usage.denied_requests += 1;
                    if let Err(error) = session.reject_question(request_id(&event)) {
                        eprintln!("[reviews] could not reject a question: {error}");
                    }
                }
                "turn.completed" => {
                    if let Some(result) = event.data.get("result").and_then(Value::as_str) {
                        result_text = Some(result.to_owned());
                    }
                    if started || result_text.is_some() {
                        done = true;
                    }
                }
                "error" if event.status.as_deref() == Some("retrying") => {}
                "error" => {
                    let detail = event.text.or(event.title).unwrap_or_else(|| "the provider reported an error".into());
                    return Err(SessionError::Provider(detail));
                }
                _ => {}
            }
        }
        if done {
            // The final message: the last thing the assistant said.
            let last = parts.into_iter().map(|(_, text)| text).filter(|text| !text.trim().is_empty()).last();
            return Ok(last.or(result_text).unwrap_or_default());
        }
    }
}

fn repair_prompt(error: &str) -> String {
    format!(
        "Your final message did not follow the output contract: {error}\n\nReply with only the corrected review: \
exactly one fenced ```json block that follows the contract. Do not use any tools."
    )
}

/// Run one reviewer session: the review turn, and one repair turn if its
/// answer does not parse. Returns the review, and the usage either way.
pub fn run(
    transport: &dyn Transport,
    harness: &str,
    start: &SessionStart<'_>,
    prompt: &str,
    cancel: &CancelToken,
    limits: Limits,
) -> (Result<RawReview, SessionError>, Usage) {
    let mut usage = Usage::default();
    let mut session = match transport.start(harness, start) {
        Ok(session) => session,
        Err(error) => return (Err(SessionError::Start(error)), usage),
    };
    let deadline = Instant::now() + limits.max_wall;
    let result = turn(session.as_mut(), prompt, &mut usage, deadline, limits, cancel).and_then(|text| match contract::parse(&text) {
        Ok(review) => Ok(review),
        Err(error) => {
            let repaired = turn(session.as_mut(), &repair_prompt(&error), &mut usage, deadline, limits, cancel)?;
            contract::parse(&repaired).map_err(SessionError::Contract)
        }
    });
    session.stop(result.is_ok());
    (result, usage)
}

// --- the prompt ---------------------------------------------------------------

/// Convention files a repository keeps for agents and contributors.
const CONVENTION_FILES: [&str; 4] = ["AGENTS.md", "CLAUDE.md", ".github/copilot-instructions.md", "CONTRIBUTING.md"];

/// The stable prefix: the same for every PR of a repository, so it caches.
pub fn instructions(checkout: &Path, user_instructions: &str, hide_attribution: bool) -> String {
    let mut sections = Vec::new();
    if hide_attribution {
        sections.push(crate::prompts::ATTRIBUTION_HIDING_RULE.to_owned());
    }
    sections.push(format!(
        "You review a pull request for Bridge. Bridge, not you, posts the review on GitHub.\n\n\
Your working directory is the pull request's head, checked out. Everything you need is in it and in \
`{CONTEXT_DIR}/`. You have read-only file tools and no network: do not run gh, git, curl or any other \
command, do not edit, create or delete files, and do not ask questions; nobody will answer. A tool or \
permission you are refused stays refused."
    ));
    let conventions: Vec<&str> = CONVENTION_FILES.iter().copied().filter(|file| checkout.join(file).is_file()).collect();
    if !conventions.is_empty() {
        sections.push(format!(
            "This repository keeps its conventions in {}. Read them before judging style or structure.",
            conventions.iter().map(|file| format!("`{file}`")).collect::<Vec<_>>().join(", ")
        ));
    }
    sections.push(format!(
        "How to review:\n{}\n\nIf these instructions mention gh, comments or posting, skip that part: \
Bridge posts what you report below.",
        user_instructions.trim()
    ));
    sections.push(OUTPUT_CONTRACT.to_owned());
    sections.join("\n\n")
}

/// What varies per PR: it goes last, in the turn.
pub fn prompt(pr: &Value) -> String {
    let number = pr["number"].as_u64().unwrap_or_default();
    let title = pr["title"].as_str().unwrap_or("");
    let changed = pr["changedFiles"].as_u64().unwrap_or_default();
    let mut text = format!(
        "Review pull request #{number}: {title}\n\n\
- `{CONTEXT_DIR}/pr.json`: the title, description, author, labels, base and head.\n\
- `{CONTEXT_DIR}/diff.patch`: the change, from the merge base to the head.\n\
- `{CONTEXT_DIR}/files.json`: the {changed} changed files with their status and line counts.\n\
- `{CONTEXT_DIR}/threads.json`: review threads already on the PR. Do not repeat what they already say.\n\n\
Read the diff, read the code around it, then answer as the contract says."
    );
    let omitted: Vec<&str> = pr["omittedFiles"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if !omitted.is_empty() {
        text.push_str(&format!(
            "\n\nThe diff was too large to include whole. These files are changed but not in diff.patch; read \
them in the checkout if they matter: {}.",
            omitted.join(", ")
        ));
    }
    text
}

// --- one pass -----------------------------------------------------------------

fn read_json(path: &Path) -> Result<Value, StepError> {
    let bytes = std::fs::read(path).map_err(|error| StepError::Fatal(format!("Could not read {}: {error}", path.display())))?;
    serde_json::from_slice(&bytes).map_err(|error| StepError::Fatal(format!("{} is not JSON: {error}", path.display())))
}

/// Copy the context folder into the checkout, where every harness can read
/// it without leaving its working directory.
fn place_context(context: &Path, checkout: &Path) -> Result<PathBuf, StepError> {
    let target = checkout.join(CONTEXT_DIR);
    std::fs::create_dir_all(&target).map_err(|error| StepError::Fatal(error.to_string()))?;
    for name in ["pr.json", "diff.patch", "files.json", "threads.json"] {
        std::fs::copy(context.join(name), target.join(name))
            .map_err(|error| StepError::Fatal(format!("Could not place {name} for the reviewer: {error}")))?;
    }
    Ok(target)
}

/// The model and effort the reviewer runs with on `harness`: the reviewer
/// settings, then the Reviewer model profile when it names this harness,
/// then the harness default (no model) at high effort.
fn model_and_effort(core: &BridgeCore, harness: &str) -> (Option<String>, String) {
    let db = super::lock_db(core);
    let settings = crate::reviewer_settings::load(&db).unwrap_or_default();
    let entry = settings.harnesses.get(harness);
    let profile = crate::model_profiles::resolve_profile(&db, &core.adapter_registry.descriptors(), crate::model_profiles::ProfilePurpose::Reviewer)
        .ok()
        .flatten();
    let model = entry
        .and_then(|entry| entry.model.clone())
        .filter(|model| !model.trim().is_empty())
        .or_else(|| profile.as_ref().filter(|profile| profile.provider == harness).map(|profile| profile.model.clone()));
    let effort = entry
        .and_then(|entry| entry.effort)
        .map(|effort| crate::reviewer_settings::effort_from_wire(effort).as_str())
        .or_else(|| profile.as_ref().map(|profile| profile.effort.as_str()))
        .unwrap_or("high");
    (model, effort.to_owned())
}

/// One reviewer pass on the real harnesses.
pub fn review_pass(core: &BridgeCore, request: PassRequest<'_>, cancel: &CancelToken) -> Result<PassOutcome, StepError> {
    let transport = RegistryTransport { registry: core.adapter_registry.clone() };
    review_pass_with(core, &transport, request, cancel, Limits::default())
}

/// [`review_pass`] over any transport.
pub fn review_pass_with(
    core: &BridgeCore,
    transport: &dyn Transport,
    request: PassRequest<'_>,
    cancel: &CancelToken,
    limits: Limits,
) -> Result<PassOutcome, StepError> {
    let checkout = request.prepared.worktree.as_deref().ok_or_else(|| StepError::Fatal("The review has no checkout".into()))?;
    let context = request.prepared.context.as_deref().ok_or_else(|| StepError::Fatal("The review has no context".into()))?;
    let placed = place_context(context, checkout)?;
    let pr = read_json(&placed.join("pr.json"))?;
    let files = read_json(&placed.join("files.json"))?;
    let patch = std::fs::read_to_string(placed.join("diff.patch")).map_err(|error| StepError::Fatal(error.to_string()))?;

    let (user_instructions, hide) = {
        let db = super::lock_db(core);
        let settings = crate::reviewer_settings::load(&db).unwrap_or_default();
        let custom = settings.system_prompt.trim().replace("{number}", &request.job.number.to_string());
        (if custom.is_empty() { REVIEW_INSTRUCTIONS.to_owned() } else { custom }, crate::attribution_settings::hide_enabled(&db))
    };
    let (model, effort) = model_and_effort(core, request.harness);
    let instructions = instructions(checkout, &user_instructions, hide);
    let start = SessionStart { cwd: checkout, model: model.as_deref(), effort: Some(&effort), instructions: &instructions };
    let (result, usage) = run(transport, request.harness, &start, &prompt(&pr), cancel, limits);
    let review = result.map_err(SessionError::into_step)?;

    let changed = files
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|file| file["path"].as_str().map(str::to_owned))
        .collect();
    let validated = contract::validate(review, &changed, &contract::hunks(&patch));
    let mut summary = validated.summary;
    let omitted: Vec<&str> = pr["omittedFiles"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if !omitted.is_empty() {
        summary.push_str(&format!("\n\nNot reviewed in the diff (too large to include): {}.", omitted.join(", ")));
    }
    Ok(PassOutcome {
        summary,
        findings: validated.findings,
        session_id: None,
        model,
        effort: Some(effort),
        tokens_in: usage.tokens_in,
        tokens_out: usage.tokens_out,
        cost_microusd: usage.cost_microusd,
        denied_requests: usage.denied_requests,
    })
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;

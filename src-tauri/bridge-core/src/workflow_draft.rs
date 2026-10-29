//! Drafting a workflow from a sentence or from a chat.
//!
//! A draft is never saved and never approved: it prefills the editor, and a
//! person reads it before anything can run. Two sources:
//!
//! * **A sentence.** A tool-less Claude turn proposes a JSON draft; a
//!   deterministic validator decides what survives. The model may only name
//!   servers Claude is configured with, may only grant writes when the
//!   sentence asks to change something, and a schedule it cannot spell is
//!   dropped to Run now. Whatever it got wrong becomes a note, not a grant.
//! * **A chat.** No model at all: the chat's first request becomes the
//!   instructions, and its grants are exactly the servers whose tools the chat
//!   actually called, read or write as they were used.

use std::io::BufRead;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use bridge_protocol::messages as wire;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::adapters::{ShutdownReason, StartRequest};
use crate::briefing_policy::BriefingRuntimePolicy;
use crate::runtime::BridgeCore;
use crate::workflow_schedule::Schedule;
use crate::workflows::{self, ToolClass};
use crate::BridgeError;

/// Hidden from chat lists, search and memory, like every session Bridge runs for itself.
pub const DRAFT_SESSION_KIND: &str = "workflow_draft";
const DRAFT_WALL_SECONDS: u64 = 60;

/// Words that ask a workflow to change something. A draft grants writes only
/// when the sentence carries one (F32a).
pub const WRITE_INTENT_WORDS: &[&str] = &[
    "post", "send", "reply", "respond", "create", "file", "open", "update", "add", "comment", "draft",
    "label", "archive", "delete", "write", "publish", "assign", "close", "merge", "move", "notify", "message",
];

pub const INSTRUCTIONS: &str = "You turn one person's description of a repeatable job into a JSON draft for a \
workflow that an AI agent will run for them. Reply with exactly one JSON object and nothing else.";

/// The model behind a text draft. A trait so tests can script it.
pub trait DraftModel {
    fn answer(&mut self, prompt: &str) -> Result<String, String>;
}

/// What drafting may know: the connector servers Claude holds, the person's
/// timezone, and the agent a draft defaults to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftContext {
    pub servers: Vec<wire::WorkflowServerOption>,
    pub timezone: String,
    pub harness: String,
    pub model: String,
}

pub fn prompt(text: &str, context: &DraftContext) -> String {
    let servers = if context.servers.is_empty() {
        "(none)".to_owned()
    } else {
        context
            .servers
            .iter()
            .map(|server| format!("- \"{}\" ({}{})", server.id, server.family, if server.connected == Some(false) { ", not connected" } else { "" }))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "The person wrote:\n\"\"\"\n{text}\n\"\"\"\n\n\
         Connector servers available (use these exact ids, and only these):\n{servers}\n\n\
         Their timezone is {timezone}.\n\n\
         Reply with this JSON object:\n\
         {{\"name\": short title, \"instructions\": the job written as clear standing instructions for the agent, \
         \"expectedResult\": one sentence on what a finished run produces, \
         \"schedule\": null for on demand, or {{\"cron\": 5-field cron, \"timezone\": IANA name}}, \
         \"servers\": [{{\"server\": exact id from the list, \"read\": true or false, \"write\": true or false}}], \
         \"needsApps\": names of apps the job needs that are not in the list}}\n\
         Grant write only if the job must change something in that app. Prefer fewer servers.",
        timezone = context.timezone,
    )
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ModelDraft {
    name: Option<String>,
    instructions: Option<String>,
    expected_result: Option<String>,
    schedule: Option<ModelSchedule>,
    servers: Vec<ModelGrant>,
    needs_apps: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ModelSchedule {
    cron: String,
    #[serde(default)]
    timezone: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ModelGrant {
    server: String,
    #[serde(default)]
    read: bool,
    #[serde(default)]
    write: bool,
}

/// The configured server a model's name refers to, or `None`.
///
/// Models often answer with the family ("slack") or drop the prefix
/// ("google calendar") instead of the exact id. Each step still resolves only
/// to a server Claude holds, and a family only when exactly one server has it,
/// so a looser spelling can never reach a server nobody configured.
fn resolve_server<'a>(name: &str, servers: &'a [wire::WorkflowServerOption]) -> Option<&'a wire::WorkflowServerOption> {
    let wanted = name.trim().to_lowercase();
    if wanted.is_empty() {
        return None;
    }
    if let Some(exact) = servers.iter().find(|server| server.id == name) {
        return Some(exact);
    }
    if let Some(folded) = servers.iter().find(|server| server.id.to_lowercase() == wanted || workflows::wire_server_name(&server.id).to_lowercase() == wanted) {
        return Some(folded);
    }
    let by_family: Vec<_> = servers.iter().filter(|server| server.family != "unknown" && server.family == wanted).collect();
    match by_family.as_slice() {
        [only] => return Some(only),
        [] => {}
        // Two servers share the family: the name is ambiguous, so it is neither.
        _ => return None,
    }
    let by_suffix: Vec<_> = servers.iter().filter(|server| server.id.to_lowercase().ends_with(&format!(" {wanted}"))).collect();
    match by_suffix.as_slice() {
        [only] => Some(only),
        _ => None,
    }
}

/// The one JSON object the model was asked for, or `None`.
fn parse_reply(reply: &str) -> Option<ModelDraft> {
    let start = reply.find('{')?;
    let end = reply.rfind('}')?;
    (start < end).then(|| serde_json::from_str(&reply[start..=end]).ok()).flatten()
}

pub fn asks_to_write(text: &str) -> bool {
    text.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| WRITE_INTENT_WORDS.contains(&word))
}

fn default_budget() -> wire::WorkBriefLimits {
    wire::WorkBriefLimits { max_wall_seconds: 600, max_turns: 24, max_tool_calls: 64, max_output_tokens: None, cost_ceiling_microusd: None }
}

fn trimmed_name(name: &str) -> String {
    let name: String = name.trim().chars().take(workflows::MAX_NAME_CHARS).collect();
    name.trim().to_owned()
}

/// Build a draft from the model's reply, keeping only what the rules allow.
fn build_text_draft(text: &str, project_id: Option<String>, reply: Option<ModelDraft>, context: &DraftContext) -> wire::WorkflowDraft {
    let mut notes = Vec::new();
    let Some(reply) = reply else {
        notes.push("Bridge could not draft this one; fill it in below.".to_owned());
        return wire::WorkflowDraft { definition: definition(text, text, None, project_id, wire::WorkflowTrigger::Manual, Vec::new(), String::new(), context), notes };
    };

    let trigger = match &reply.schedule {
        None => wire::WorkflowTrigger::Manual,
        Some(schedule) => {
            let timezone = schedule.timezone.clone().filter(|zone| !zone.trim().is_empty()).unwrap_or_else(|| context.timezone.clone());
            match Schedule::parse(&schedule.cron, &timezone) {
                Ok(_) => wire::WorkflowTrigger::Schedule { cron: schedule.cron.trim().to_owned(), timezone },
                Err(reason) => {
                    notes.push(format!("The schedule it suggested does not work ({reason}); it runs on Run now until you set one."));
                    wire::WorkflowTrigger::Manual
                }
            }
        }
    };

    let may_write = asks_to_write(text);
    let mut grants: Vec<wire::WorkflowGrant> = Vec::new();
    let mut held_back_writes = false;
    for grant in reply.servers {
        let Some(server) = resolve_server(&grant.server, &context.servers) else {
            notes.push(format!("\"{}\" is not a connector Claude is configured with, so it was left out.", grant.server));
            continue;
        };
        if server.connected == Some(false) {
            notes.push(format!("Connect {} in Claude first; it was left out.", server.id));
            continue;
        }
        let write = if grant.write && may_write { wire::WorkflowWriteAccess::Ask } else { wire::WorkflowWriteAccess::Never };
        held_back_writes |= grant.write && !may_write;
        if !grant.read && write == wire::WorkflowWriteAccess::Never {
            continue;
        }
        match grants.iter_mut().find(|existing| existing.server == server.id) {
            Some(existing) => {
                existing.read |= grant.read;
                if write == wire::WorkflowWriteAccess::Ask {
                    existing.write = write;
                }
            }
            None => grants.push(wire::WorkflowGrant { server: server.id.clone(), read: grant.read, write }),
        }
    }
    if held_back_writes {
        notes.push("Writes are off: your description doesn't ask it to change anything. Turn them on below if it should.".into());
    }
    for app in reply.needs_apps {
        let lowered = app.to_lowercase();
        let held = context.servers.iter().any(|server| server.family == lowered || server.id.to_lowercase().contains(&lowered));
        if !held && !lowered.trim().is_empty() {
            notes.push(format!("Connect {} in Claude first to use it.", app.trim()));
        }
    }

    let name = reply.name.as_deref().map(trimmed_name).filter(|name| !name.is_empty());
    let instructions = reply.instructions.filter(|value| !value.trim().is_empty()).unwrap_or_else(|| text.to_owned());
    wire::WorkflowDraft {
        definition: definition(
            name.as_deref().unwrap_or(text),
            &instructions,
            None,
            project_id,
            trigger,
            grants,
            reply.expected_result.unwrap_or_default().trim().chars().take(workflows::MAX_EXPECTED_RESULT_CHARS).collect(),
            context,
        ),
        notes,
    }
}

#[allow(clippy::too_many_arguments)]
fn definition(
    name: &str,
    instructions: &str,
    harness_model: Option<(String, String)>,
    project_id: Option<String>,
    trigger: wire::WorkflowTrigger,
    grants: Vec<wire::WorkflowGrant>,
    expected_result: String,
    context: &DraftContext,
) -> wire::WorkflowDefinition {
    let (harness, model) = harness_model.unwrap_or_else(|| (context.harness.clone(), context.model.clone()));
    wire::WorkflowDefinition {
        name: trimmed_name(name),
        project_id,
        steps: vec![wire::WorkflowStep {
            harness: wire::HarnessId::parse(&harness).unwrap_or_else(|_| wire::HarnessId::parse("claude").expect("claude is a harness id")),
            model,
            effort: None,
            instructions: instructions.trim().to_owned(),
        }],
        trigger,
        grants,
        always_allow: Vec::new(),
        files: wire::WorkflowFiles::None,
        budget: default_budget(),
        expected_result,
        notify: wire::WorkflowNotify::default(),
    }
}

/// Draft from a sentence with whatever model is given.
pub fn draft_from_text(
    text: &str,
    project_id: Option<String>,
    context: &DraftContext,
    model: &mut dyn DraftModel,
) -> Result<wire::WorkflowDraft, BridgeError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(BridgeError::Invalid("Describe the job first".into()));
    }
    let reply = model.answer(&prompt(text, context)).ok().and_then(|reply| parse_reply(&reply));
    Ok(build_text_draft(text, project_id, reply, context))
}

/// Draft from a chat: its first request, its agent, and the servers it used.
pub fn draft_from_session(
    db: &Connection,
    session_id: &str,
    configured_servers: &[String],
    context: &DraftContext,
) -> Result<wire::WorkflowDraft, BridgeError> {
    let row: Option<(String, Option<String>, Option<String>, String, Option<String>)> = db
        .query_row(
            "SELECT harness, model, title, label, kind FROM sessions WHERE id=?1",
            params![session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .optional()?;
    let Some((harness, model, title, label, kind)) = row else {
        return Err(BridgeError::Invalid(format!("chat {session_id} does not exist")));
    };
    let kind = kind.unwrap_or_default();
    if kind == "worker" || crate::work_briefing_config::is_hidden_session_kind(Some(&kind)) {
        return Err(BridgeError::Invalid("Only a chat you started can become a workflow".into()));
    }
    let first_request: Option<String> = db
        .query_row(
            "SELECT json_extract(payload,'$.text') FROM session_entries
             WHERE session_id=?1 AND kind='user.message' AND COALESCE(json_extract(payload,'$.text'),'')<>''
             ORDER BY sequence LIMIT 1",
            params![session_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(first_request) = first_request else {
        return Err(BridgeError::Invalid("This chat has no request to turn into a workflow yet".into()));
    };

    let mut statement = db.prepare(
        "SELECT COALESCE(json_extract(payload,'$.title'), json_extract(payload,'$.data.name'), json_extract(payload,'$.data.toolName'))
         FROM session_entries WHERE session_id=?1 AND kind='tool.started' ORDER BY sequence",
    )?;
    let tools: Vec<String> = statement
        .query_map(params![session_id], |row| row.get::<_, Option<String>>(0))?
        .filter_map(|row| row.ok().flatten())
        .collect();
    let mut grants: Vec<wire::WorkflowGrant> = Vec::new();
    for identity in &tools {
        let Some((segment, _)) = workflows::split_identity(identity) else { continue };
        let Some(server) = configured_servers.iter().find(|server| workflows::wire_server_name(server) == segment) else { continue };
        let write = workflows::tool_class(identity) == ToolClass::Write;
        match grants.iter_mut().find(|grant| &grant.server == server) {
            Some(grant) => {
                grant.read |= !write;
                if write {
                    grant.write = wire::WorkflowWriteAccess::Ask;
                }
            }
            None => grants.push(wire::WorkflowGrant {
                server: server.clone(),
                read: !write,
                write: if write { wire::WorkflowWriteAccess::Ask } else { wire::WorkflowWriteAccess::Never },
            }),
        }
    }
    let name = title.filter(|title| !title.trim().is_empty()).unwrap_or(label);
    let notes = vec![format!(
        "Made from the chat \"{}\". Check the instructions read as a standing job, then set when it runs.",
        name.trim()
    )];
    Ok(wire::WorkflowDraft {
        definition: definition(
            &name,
            &first_request,
            Some((harness, model.unwrap_or_else(|| context.model.clone()))),
            None,
            wire::WorkflowTrigger::Manual,
            grants,
            String::new(),
            context,
        ),
        notes,
    })
}

// --- the live model ------------------------------------------------------------------

/// One tool-less Claude turn in a hidden session, started per draft.
pub struct ClaudeDraftModel {
    core: Arc<BridgeCore>,
    model: String,
}

impl ClaudeDraftModel {
    pub fn new(core: &Arc<BridgeCore>, model: &str) -> Self {
        Self { core: core.clone(), model: model.to_owned() }
    }
}

impl DraftModel for ClaudeDraftModel {
    fn answer(&mut self, prompt: &str) -> Result<String, String> {
        let limits = wire::WorkBriefLimits { max_wall_seconds: DRAFT_WALL_SECONDS as i64, max_turns: 1, max_tool_calls: 1, max_output_tokens: None, cost_ceiling_microusd: None };
        let policy = BriefingRuntimePolicy::compile_toolless(limits).map_err(|refusal| refusal.reason())?;
        let session_id = Uuid::new_v4().to_string();
        let scratch = self.core.chat_scratch_dir(&session_id);
        std::fs::create_dir_all(&scratch).map_err(|error| error.to_string())?;
        let cwd = scratch.to_string_lossy().to_string();
        self.core
            .db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,model,kind,title,cwd,depth)
                 VALUES(?1,NULL,'claude','Draft','working','reported',?2,?3,'Workflow draft',?4,0)",
                params![session_id, self.model, DRAFT_SESSION_KIND, cwd],
            )
            .map_err(|error| error.to_string())?;
        let result = (|| {
            let started = self
                .core
                .adapter_registry
                .start(
                    "claude",
                    StartRequest {
                        cwd: &cwd,
                        model: Some(&self.model),
                        effort: None,
                        instructions: Some(INSTRUCTIONS),
                        write_mode: None,
                        read_only_sandbox: None,
                        briefing: Some(&policy),
                        interactive: false,
                        on_progress: None,
                    },
                )
                .map_err(|error| format!("provider start: {error}"))?;
            let (sender, lines) = mpsc::channel::<String>();
            let mut reader = started.reader;
            std::thread::spawn(move || {
                let mut line = String::new();
                while matches!(reader.read_line(&mut line), Ok(read) if read > 0) {
                    if sender.send(line.trim_end().to_owned()).is_err() {
                        break;
                    }
                    line.clear();
                }
            });
            let mut runtime = started.runtime;
            let outcome = (|| {
                runtime.send_turn(prompt).map_err(|error| format!("send: {error}"))?;
                let deadline = Instant::now() + Duration::from_secs(DRAFT_WALL_SECONDS);
                let mut reply = String::new();
                loop {
                    let wait = deadline.saturating_duration_since(Instant::now());
                    if wait.is_zero() {
                        return Err("the draft took too long".to_owned());
                    }
                    match lines.recv_timeout(wait.min(Duration::from_millis(250))) {
                        Ok(line) => {
                            let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
                            match message.get("type").and_then(Value::as_str) {
                                Some("assistant") => {
                                    let blocks = message.pointer("/message/content").and_then(Value::as_array);
                                    for block in blocks.into_iter().flatten() {
                                        if block.get("type").and_then(Value::as_str) == Some("text") {
                                            reply.push_str(block.get("text").and_then(Value::as_str).unwrap_or_default());
                                        }
                                    }
                                }
                                Some("result") => {
                                    let _ = crate::policy::record_provider_usage(
                                        &self.core.db.lock().unwrap(),
                                        &session_id,
                                        &session_id,
                                        None,
                                        "provider.claude",
                                        &message,
                                    );
                                    if reply.trim().is_empty() {
                                        reply.push_str(message.get("result").and_then(Value::as_str).unwrap_or_default());
                                    }
                                    return Ok(reply);
                                }
                                _ => {}
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => return Err("the provider ended before answering".to_owned()),
                    }
                }
            })();
            runtime.stop(if outcome.is_ok() { ShutdownReason::Completed } else { ShutdownReason::Failed });
            outcome
        })();
        let _ = self.core.db.lock().unwrap().execute(
            "UPDATE sessions SET status=?2 WHERE id=?1",
            params![session_id, if result.is_ok() { "ended" } else { "failed" }],
        );
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scripted(Result<String, String>);
    impl DraftModel for Scripted {
        fn answer(&mut self, _prompt: &str) -> Result<String, String> {
            self.0.clone()
        }
    }

    pub(crate) fn context() -> DraftContext {
        DraftContext {
            servers: vec![
                wire::WorkflowServerOption { id: "claude.ai Slack".into(), family: "slack".into(), connected: Some(true) },
                wire::WorkflowServerOption { id: "claude.ai Linear".into(), family: "linear".into(), connected: Some(true) },
                wire::WorkflowServerOption { id: "claude.ai Gmail".into(), family: "gmail".into(), connected: Some(false) },
            ],
            timezone: "Asia/Kolkata".into(),
            harness: "claude".into(),
            model: "haiku".into(),
        }
    }

    fn draft(text: &str, reply: &str) -> wire::WorkflowDraft {
        draft_from_text(text, None, &context(), &mut Scripted(Ok(reply.into()))).unwrap()
    }

    #[test]
    fn a_draft_is_never_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::store::open(&dir.path().join("bridge.db")).unwrap();
        let _ = draft("Every weekday at 9 summarise my Slack mentions", r#"{"name":"Morning brief","schedule":{"cron":"0 9 * * 1-5"},"servers":[{"server":"claude.ai Slack","read":true}]}"#);
        let saved: i64 = db.query_row("SELECT COUNT(*) FROM workflows", [], |row| row.get(0)).unwrap();
        assert_eq!(saved, 0, "drafting is a pure function of the reply; nothing touches the store");
    }

    #[test]
    fn an_unconnected_app_becomes_a_connect_first_note() {
        let result = draft(
            "Every Friday at 4 find unpaid invoice emails and summarise them",
            r#"{"name":"Invoice chaser","schedule":{"cron":"0 16 * * 5"},"servers":[{"server":"claude.ai Gmail","read":true}],"needsApps":["QuickBooks"]}"#,
        );
        assert!(result.definition.grants.is_empty());
        assert!(result.notes.iter().any(|note| note.contains("Connect claude.ai Gmail in Claude first")));
        assert!(result.notes.iter().any(|note| note.contains("Connect QuickBooks in Claude first")));
    }

    #[test]
    fn a_draft_grants_writes_only_when_the_sentence_asks_for_one() {
        let reply = r#"{"servers":[{"server":"claude.ai Slack","read":true,"write":true}]}"#;
        let reading = draft("Every morning summarise my Slack mentions", reply);
        assert_eq!(reading.definition.grants[0].write, wire::WorkflowWriteAccess::Never);
        assert!(reading.notes.iter().any(|note| note.contains("Writes are off")));
        let posting = draft("Every morning post a summary of my mentions to #team", reply);
        assert_eq!(posting.definition.grants[0].write, wire::WorkflowWriteAccess::Ask);
        assert!(!posting.notes.iter().any(|note| note.contains("Writes are off")));
    }

    #[test]
    fn an_unparseable_schedule_becomes_run_now_with_a_note() {
        let result = draft("every 5 minutes check Slack", r#"{"schedule":{"cron":"*/5 * * * *"}}"#);
        assert_eq!(result.definition.trigger, wire::WorkflowTrigger::Manual);
        assert!(result.notes.iter().any(|note| note.contains("15 minutes")));
        let bad_zone = draft("every day at 9", r#"{"schedule":{"cron":"0 9 * * *","timezone":"Mars/Base"}}"#);
        assert_eq!(bad_zone.definition.trigger, wire::WorkflowTrigger::Manual);
        let defaulted = draft("every day at 9", r#"{"schedule":{"cron":"0 9 * * *"}}"#);
        assert_eq!(defaulted.definition.trigger, wire::WorkflowTrigger::Schedule { cron: "0 9 * * *".into(), timezone: "Asia/Kolkata".into() });
    }

    #[test]
    fn the_draft_parser_takes_only_the_json_object_it_asked_for() {
        let wrapped = draft("summarise Linear", "Here you go:\n```json\n{\"name\":\"Linear digest\",\"servers\":[{\"server\":\"claude.ai Linear\",\"read\":true}]}\n```");
        assert_eq!(wrapped.definition.name, "Linear digest");
        assert_eq!(wrapped.definition.grants.len(), 1);
        let garbage = draft("summarise Linear", "I can't help with that.");
        assert_eq!(garbage.definition.steps[0].instructions, "summarise Linear");
        assert!(garbage.notes.iter().any(|note| note.contains("could not draft")));
        let failed = draft_from_text("summarise Linear", None, &context(), &mut Scripted(Err("offline".into()))).unwrap();
        assert!(failed.definition.grants.is_empty());
        assert!(draft_from_text("   ", None, &context(), &mut Scripted(Ok("{}".into()))).is_err());
    }

    #[test]
    fn a_family_or_short_name_resolves_only_to_one_configured_server() {
        let mut servers = context().servers;
        servers.push(wire::WorkflowServerOption { id: "claude.ai Google Calendar".into(), family: "unknown".into(), connected: Some(true) });
        assert_eq!(resolve_server("slack", &servers).map(|server| server.id.as_str()), Some("claude.ai Slack"));
        assert_eq!(resolve_server("claude_ai_Linear", &servers).map(|server| server.id.as_str()), Some("claude.ai Linear"));
        assert_eq!(resolve_server("Google Calendar", &servers).map(|server| server.id.as_str()), Some("claude.ai Google Calendar"));
        assert_eq!(resolve_server("calendar", &servers).map(|server| server.id.as_str()), Some("claude.ai Google Calendar"));
        assert!(resolve_server("github", &servers).is_none());
        // Two servers of one family: a family name is ambiguous and resolves to neither.
        servers.push(wire::WorkflowServerOption { id: "claude.ai Slack (work)".into(), family: "slack".into(), connected: Some(true) });
        assert!(resolve_server("slack", &servers).is_none());
    }

    #[test]
    fn a_draft_never_names_a_server_claude_is_not_configured_with() {
        let result = draft("summarise Notion", r#"{"servers":[{"server":"notion","read":true},{"server":"claude.ai Linear","read":true}]}"#);
        assert_eq!(result.definition.grants.iter().map(|grant| grant.server.as_str()).collect::<Vec<_>>(), vec!["claude.ai Linear"]);
        assert!(result.notes.iter().any(|note| note.contains("\"notion\" is not a connector")));
    }

    // --- the C14 eval -------------------------------------------------------------

    #[derive(serde::Deserialize, serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct EvalFile {
        context: EvalContext,
        bar: EvalBar,
        cases: Vec<EvalCase>,
    }
    #[derive(serde::Deserialize, serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct EvalContext {
        servers: Vec<wire::WorkflowServerOption>,
        timezone: String,
        harness: String,
        model: String,
    }
    #[derive(serde::Deserialize, serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct EvalBar {
        min_trigger_exact: usize,
        max_over_grants: usize,
        max_under_grants: usize,
    }
    #[derive(serde::Deserialize, serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct EvalCase {
        sentence: String,
        expected: Expected,
        reply: Option<String>,
    }
    #[derive(serde::Deserialize, serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Expected {
        cron: Option<String>,
        grants: Vec<ExpectedGrant>,
    }
    #[derive(serde::Deserialize, serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct ExpectedGrant {
        server: String,
        write: bool,
    }

    const EVAL_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testing/fixtures/workflow-drafts/cases.json");

    fn eval_context(file: &EvalFile) -> DraftContext {
        DraftContext {
            servers: file.context.servers.clone(),
            timezone: file.context.timezone.clone(),
            harness: file.context.harness.clone(),
            model: file.context.model.clone(),
        }
    }

    /// (trigger exact, over-grants, under-grants, per-case misses)
    fn score(file: &EvalFile) -> (usize, usize, usize, Vec<String>) {
        let context = eval_context(file);
        let (mut exact, mut over, mut under, mut misses) = (0, 0, 0, Vec::new());
        for case in &file.cases {
            let reply = case.reply.clone().unwrap_or_default();
            let draft = draft_from_text(&case.sentence, None, &context, &mut Scripted(Ok(reply))).unwrap();
            let cron = match &draft.definition.trigger {
                wire::WorkflowTrigger::Schedule { cron, .. } => Some(cron.split_whitespace().collect::<Vec<_>>().join(" ")),
                wire::WorkflowTrigger::Manual => None,
            };
            if cron == case.expected.cron {
                exact += 1;
            } else {
                misses.push(format!("trigger: {:?} got {cron:?} for \"{}\"", case.expected.cron, case.sentence));
            }
            for grant in &draft.definition.grants {
                let expected = case.expected.grants.iter().find(|expected| expected.server == grant.server);
                let wrote = grant.write == wire::WorkflowWriteAccess::Ask;
                if expected.is_none() || (wrote && !expected.unwrap().write) {
                    over += 1;
                    misses.push(format!("over: {} (write {wrote}) for \"{}\"", grant.server, case.sentence));
                }
            }
            for expected in &case.expected.grants {
                let got = draft.definition.grants.iter().find(|grant| grant.server == expected.server);
                if got.is_none() || (expected.write && got.unwrap().write != wire::WorkflowWriteAccess::Ask) {
                    under += 1;
                    misses.push(format!("under: {} (write {}) for \"{}\"", expected.server, expected.write, case.sentence));
                }
            }
        }
        (exact, over, under, misses)
    }

    #[test]
    fn recorded_drafts_meet_the_eval_bar() {
        let file: EvalFile = serde_json::from_str(&std::fs::read_to_string(EVAL_PATH).unwrap()).unwrap();
        assert_eq!(file.cases.len(), 30);
        assert!(
            file.cases.iter().all(|case| case.reply.is_some()),
            "record replies with: cargo test -p bridge-core --lib record_live_draft_replies -- --ignored"
        );
        let (exact, over, under, misses) = score(&file);
        eprintln!("workflow draft eval: triggers exact {exact}/30, over-grants {over}, under-grants {under}");
        assert!(over <= file.bar.max_over_grants, "over-grants {over}: {misses:#?}");
        assert!(exact >= file.bar.min_trigger_exact, "triggers exact {exact}/30: {misses:#?}");
        assert!(under <= file.bar.max_under_grants, "under-grants {under}: {misses:#?}");
    }

    /// Live: asks the real model for every case and records its reply. Spends
    /// roughly 30 small Haiku turns, so it is run by hand before merge:
    /// `cargo test -p bridge-core --lib record_live_draft_replies -- --ignored --nocapture`.
    #[test]
    #[ignore = "calls the real model"]
    fn record_live_draft_replies() {
        let dir = tempfile::tempdir().unwrap();
        let core = Arc::new(
            crate::runtime::BridgeCore::boot(crate::BootConfig {
                data_dir: dir.path().to_path_buf(),
                browser_extension_path: dir.path().join("no-extension"),
                events: None,
            })
            .unwrap(),
        );
        let mut file: EvalFile = serde_json::from_str(&std::fs::read_to_string(EVAL_PATH).unwrap()).unwrap();
        let context = eval_context(&file);
        let mut model = ClaudeDraftModel::new(&core, &file.context.model);
        for case in file.cases.iter_mut() {
            case.reply = Some(model.answer(&prompt(&case.sentence, &context)).unwrap_or_else(|error| format!("ERROR: {error}")));
        }
        std::fs::write(EVAL_PATH, serde_json::to_string_pretty(&file).unwrap() + "\n").unwrap();
        let (exact, over, under, misses) = score(&file);
        eprintln!("triggers exact {exact}/30, over-grants {over}, under-grants {under}\n{misses:#?}");
    }

    #[test]
    fn save_as_workflow_grants_only_tools_the_chat_called() {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::store::open(&dir.path().join("bridge.db")).unwrap();
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,model,kind,title,depth) VALUES('chat',NULL,'claude','Chat','ready','reported','sonnet','direct','Release notes',0)",
            [],
        )
        .unwrap();
        let entry = |sequence: i64, kind: &str, payload: serde_json::Value| {
            db.execute(
                "INSERT INTO session_entries(id,session_id,sequence,kind,payload,created_at) VALUES(?1,'chat',?2,?3,?4,'now')",
                params![format!("e{sequence}"), sequence, kind, payload.to_string()],
            )
            .unwrap();
        };
        entry(1, "user.message", serde_json::json!({ "role": "user", "text": "Write release notes from merged PRs and post them to #releases" }));
        entry(2, "tool.started", serde_json::json!({ "title": "mcp__claude_ai_Slack__slack_search_messages" }));
        entry(3, "tool.started", serde_json::json!({ "title": "mcp__claude_ai_Slack__slack_send_message" }));
        entry(4, "tool.started", serde_json::json!({ "title": "Bash" }));
        entry(5, "tool.started", serde_json::json!({ "title": "mcp__github__list_pull_requests" }));
        entry(6, "user.message", serde_json::json!({ "role": "user", "text": "thanks" }));
        let servers = vec!["claude.ai Slack".to_owned(), "claude.ai Linear".to_owned()];
        let result = draft_from_session(&db, "chat", &servers, &context()).unwrap();
        assert_eq!(result.definition.name, "Release notes");
        assert_eq!(result.definition.steps[0].instructions, "Write release notes from merged PRs and post them to #releases");
        assert_eq!(result.definition.steps[0].model, "sonnet");
        assert_eq!(
            result.definition.grants,
            vec![wire::WorkflowGrant { server: "claude.ai Slack".into(), read: true, write: wire::WorkflowWriteAccess::Ask }],
            "only configured servers the chat called; Linear was never used, GitHub is not configured"
        );

        db.execute("UPDATE sessions SET kind='workflow' WHERE id='chat'", []).unwrap();
        assert!(draft_from_session(&db, "chat", &servers, &context()).is_err(), "a workflow run is not a chat to save");
    }
}

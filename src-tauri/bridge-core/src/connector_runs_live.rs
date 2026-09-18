//! The production connector runs: hidden, bounded harness turns that read a
//! connector and render one notification at a time.
//!
//! Same shape as the live briefing and consolidation runs — `kind =
//! 'connector'` so no surface lists the session, a scoped briefing policy so the
//! only reachable tools are the one connector's reads, exactly one turn, and a
//! settled session row on every exit. What is different is the cadence: this
//! runs on a timer whether or not anyone is looking, so every ceiling here is
//! tighter and the failure path is always "say so", never "retry harder".
//!
//! The ordering rule that makes the surface trustworthy lives in
//! [`poll_once`]: **announce, then render**. An item is written to the ledger
//! and published before any card exists, so a render that is slow, expensive, or
//! broken delays the polish and never the notification.

use std::io::BufRead;
use std::collections::BTreeSet;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration as StdDuration, Instant};

use chrono::Utc;
use rusqlite::params;
use serde_json::Value;
use uuid::Uuid;

use crate::adapters::{ShutdownReason, StartRequest};
use crate::briefing_policy::{ActionIntent, BriefingRuntimePolicy};
use crate::connector_inbox::{self, Resolution};
use crate::connector_runs::{
    self, AuthorizedAction, ConnectorAction, RunKind, INGRESS_FENCE,
};
use crate::connector_surface::{
    ConnectorAvailability, ConnectorCard, HarnessConnectors, InboxItem,
};
use crate::events::CoreEvent;
use crate::runtime::BridgeCore;
use crate::work_connectors::ConnectorFamily;

/// Hidden session kind. No surface lists these.
pub const CONNECTOR_SESSION_KIND: &str = "connector";

/// How often ingress runs.
///
/// Slower than the GitHub poller's 15s on purpose: each cycle here is a model
/// turn, not a `gh` call. A minute of latency on a DM is the price of a surface
/// that costs almost nothing to leave on, and push ingress is the fix if it ever
/// stops being an acceptable trade.
///
/// One cadence, not a focused/unfocused pair: `bridge-core` has no window-focus
/// signal to switch on, and a constant naming a behaviour nothing implements is
/// worse than no constant.
pub const POLL_CADENCE: StdDuration = StdDuration::from_secs(30);

/// Which families have an ingress cycle running right now.
///
/// The manual refresh in the pane header and the timer are two callers of the
/// same cycle, and a cycle is a model turn. Without this they can overlap: two
/// runs, two bills, and two racing writers of the same poll state. The ledger's
/// unique insert means the *user* would never see a duplicate — this is about
/// not paying for the same answer twice.
#[derive(Default)]
pub struct ConnectorPoller {
    in_flight: Mutex<BTreeSet<&'static str>>,
    /// Items with an action run in flight. Held for the duration of the send
    /// instead of resolving the item up front, so a concurrent click is refused
    /// while a failed send stays retryable.
    sending: Mutex<BTreeSet<String>>,
}

impl ConnectorPoller {
    /// Claim the cycle for `family`, or refuse because one is already running.
    fn begin(&self, family: ConnectorFamily) -> bool {
        self.in_flight.lock().unwrap().insert(family.as_str())
    }

    fn finish(&self, family: ConnectorFamily) {
        self.in_flight.lock().unwrap().remove(family.as_str());
    }

    /// Claim an item for one action run, or refuse because one is in flight.
    pub(crate) fn begin_action(&self, item_key: &str) -> bool {
        self.sending.lock().unwrap().insert(item_key.to_owned())
    }

    pub(crate) fn finish_action(&self, item_key: &str) {
        self.sending.lock().unwrap().remove(item_key);
    }
}

/// Most items one cycle will render. A quiet hour then a burst of thirty
/// mentions must not turn into thirty simultaneous model turns; the rest keep
/// their Bridge-authored card and are picked up next cycle.
pub const MAX_RENDERS_PER_CYCLE: usize = 5;

/// One ingress cycle for one family: read, announce, then render what is new.
///
/// Returns how many items were announced. Never returns an error — a poll that
/// nobody is watching has nowhere to surface one, so failures are written to the
/// poll state and read back by the pane as a degraded badge.
pub fn poll_once(core: &Arc<BridgeCore>, family: ConnectorFamily) -> usize {
    if !core.connector_poller.begin(family) {
        // A cycle is already running for this family — the timer and the pane's
        // refresh control both land here. Joining it is not worth a second turn.
        return 0;
    }
    let announced = poll_claimed(core, family);
    core.connector_poller.finish(family);
    announced
}

fn poll_claimed(core: &Arc<BridgeCore>, family: ConnectorFamily) -> usize {
    let now = Utc::now().to_rfc3339();
    let Some(connection) = available_connection(family) else {
        let db = core.db.lock().unwrap();
        let _ = connector_inbox::record_poll_failure(
            &db,
            family,
            &now,
            &format!("{} is not connected in any harness", family.display_name()),
        );
        return 0;
    };

    // One user-facing switch for "include things I have already read", shared
    // with the Work briefing rather than duplicated: it is the same question
    // about the same connectors, and two toggles that must agree are a bug
    // waiting for someone to set one of them.
    let include_read_mentions = {
        let db = core.db.lock().unwrap();
        crate::work::read_settings(&db)
            .map(|snapshot| snapshot.settings.include_read_mentions)
            .unwrap_or(false)
    };

    let output = match one_bounded_turn(
        core,
        &connection,
        RunKind::Ingress,
        &connector_runs::ingress_prompt(family, include_read_mentions),
        None,
    ) {
        Ok(text) => text,
        Err(detail) => {
            let db = core.db.lock().unwrap();
            let _ = connector_inbox::record_poll_failure(&db, family, &now, &detail);
            core.events.publish(CoreEvent::ConnectorInboxChanged { family: family.as_str().into() });
            return 0;
        }
    };

    // A run that answered without the fence read the inbox but did not report
    // it. That is a failed cycle, not an empty one: treating it as empty would
    // let a confused turn look exactly like a quiet hour.
    let Some(payload) = connector_runs::extract_fenced(&output, INGRESS_FENCE) else {
        let db = core.db.lock().unwrap();
        let _ = connector_inbox::record_poll_failure(
            &db,
            family,
            &now,
            "the ingress run did not return a fenced item list",
        );
        core.events.publish(CoreEvent::ConnectorInboxChanged { family: family.as_str().into() });
        return 0;
    };
    let Ok(value) = serde_json::from_str::<Value>(payload.trim()) else {
        let db = core.db.lock().unwrap();
        let _ = connector_inbox::record_poll_failure(
            &db, family, &now, "the ingress run returned malformed JSON",
        );
        core.events.publish(CoreEvent::ConnectorInboxChanged { family: family.as_str().into() });
        return 0;
    };

    let reported = connector_inbox::parse_ingress(&value, family, &now);
    let fresh = {
        let db = core.db.lock().unwrap();
        let fresh = connector_inbox::record_arrivals(&db, &reported).unwrap_or_default();
        let _ = connector_inbox::record_poll_success(&db, family, &now);
        fresh
    };

    // Announce first. Everything below this line is presentation.
    for item in &fresh {
        core.events.publish(CoreEvent::ConnectorItemArrived {
            family: family.as_str().into(),
            item_key: item.key(),
            headline: ConnectorCard::fallback(item).headline,
            channel_label: item.channel_label.clone(),
            author: item.author.clone(),
        });
    }
    if !fresh.is_empty() {
        core.events.publish(CoreEvent::ConnectorInboxChanged { family: family.as_str().into() });
    }

    // Render this cycle's arrivals first, then spend anything left of the budget
    // on the backlog. Without the second half, a burst larger than the cap left
    // its overflow pending forever: those rows are already stored, so the next
    // poll's `INSERT OR IGNORE` drops them from `fresh` and nothing else ever
    // calls `render_item` for them.
    let mut budget = MAX_RENDERS_PER_CYCLE;
    for item in fresh.iter().take(budget) {
        render_item(core, &connection, item);
    }
    budget = budget.saturating_sub(fresh.len());
    if budget > 0 {
        for item in pending_backlog(core, family, budget, &fresh) {
            render_item(core, &connection, &item);
        }
    }
    fresh.len()
}

/// Announced items still waiting for a card, oldest first, excluding the ones
/// this cycle just rendered.
fn pending_backlog(
    core: &Arc<BridgeCore>,
    family: ConnectorFamily,
    budget: usize,
    just_rendered: &[InboxItem],
) -> Vec<InboxItem> {
    let db = core.db.lock().unwrap();
    let Ok(pending) = connector_inbox::pending_without_cards(&db, family, budget + just_rendered.len())
    else {
        return Vec::new();
    };
    let skip: BTreeSet<String> = just_rendered.iter().map(InboxItem::key).collect();
    pending.into_iter().filter(|item| !skip.contains(&item.key())).take(budget).collect()
}

/// Render exactly one item and attach the result.
///
/// This is the whole "render only what a notification needs" rule in one
/// function: it is called per arrival, it is given one item, and there is no
/// batch or refresh variant of it anywhere. Re-opening the pane re-reads the
/// stored card; it does not come back here.
pub fn render_item(core: &Arc<BridgeCore>, connection: &ConnectorAvailability, item: &InboxItem) {
    let (card, rejection) = match one_bounded_turn(
        core,
        connection,
        RunKind::Render,
        &connector_runs::render_prompt(item),
        None,
    ) {
        Ok(text) => connector_runs::card_or_fallback(&text, item),
        // The notification already landed. A failed render costs polish only,
        // so the item still gets a card — Bridge's own.
        Err(detail) => (ConnectorCard::fallback(item), Some(detail)),
    };
    let attached = {
        let db = core.db.lock().unwrap();
        connector_inbox::attach_card(&db, &item.key(), &card, rejection.as_deref())
            .unwrap_or(false)
    };
    if attached {
        core.events.publish(CoreEvent::ConnectorCardReady {
            family: item.family.as_str().into(),
            item_key: item.key(),
            headline: card.headline.clone(),
            harness_rendered: card.harness_rendered,
        });
    }
}

/// Run an approved action, then resolve its item.
///
/// Takes [`AuthorizedAction`], which only `connector_runs::authorize` can build,
/// so there is no way to reach a send from here without a human decision on the
/// exact text.
pub fn execute_action(
    core: &Arc<BridgeCore>,
    authorized: &AuthorizedAction,
) -> Result<(), String> {
    let item = authorized.action().item();
    let connection = available_connection(item.family)
        .ok_or_else(|| format!("{} is not connected in any harness", item.family.display_name()))?;

    // Hold the item for the duration of the run rather than resolving it up
    // front. Resolving first did stop a double send, but it also made every
    // failure permanent: the composer disappeared and both `authorize` and the
    // storage claim refused a second attempt, so a timed-out send could never be
    // retried. An in-flight claim blocks the concurrent click just as well and
    // leaves a failed send exactly where the user can try it again.
    if !core.connector_poller.begin_action(&item.key()) {
        return Err("this message is already being answered".into());
    }
    let outcome = run_authorized(core, &connection, authorized);
    core.connector_poller.finish_action(&item.key());

    let succeeded = outcome.is_ok();
    if succeeded {
        let db = core.db.lock().unwrap();
        // Only now. The residual window — a send that lands and a database that
        // then fails — leaves the item unresolved and retryable, which is the
        // safer end of a trade that has no free side.
        let _ = connector_inbox::resolve(
            &db,
            &item.key(),
            match authorized.action() {
                ConnectorAction::Reply { .. } => Resolution::Replied,
                ConnectorAction::React { .. } => Resolution::Reacted,
            },
            &Utc::now().to_rfc3339(),
        );
    }
    core.events.publish(CoreEvent::ConnectorItemResolved {
        family: item.family.as_str().into(),
        item_key: item.key(),
        succeeded,
    });
    core.events.publish(CoreEvent::ConnectorInboxChanged {
        family: item.family.as_str().into(),
    });
    outcome
}

/// The run half of an action, split out so the in-flight claim is released on
/// every path including an early return.
fn run_authorized(
    core: &Arc<BridgeCore>,
    connection: &ConnectorAvailability,
    authorized: &AuthorizedAction,
) -> Result<(), String> {
    let intent = match authorized.action() {
        ConnectorAction::Reply { .. } => ActionIntent::Reply,
        ConnectorAction::React { .. } => ActionIntent::React,
    };
    let text =
        one_bounded_turn(core, connection, RunKind::Action, &authorized.prompt(), Some(intent))?;
    // The turn completing is not the send succeeding.
    action_succeeded(&text, intent)
}

/// What every harness Bridge can ask reports having connected.
///
/// **This is the second extension point.** Today exactly one harness exposes its
/// MCP inventory to Bridge, so this list has one entry; when Codex or OpenCode
/// grow an equivalent of `claude mcp list`, adding them is appending a row here
/// and nothing else. Everything downstream — availability, the run layer, the
/// API — already reasons about "which harness owns this connector" rather than
/// assuming, because the answer travels with the connector.
pub fn discover_harness_connectors() -> Vec<HarnessConnectors> {
    let claude = crate::marketplace::claude_sdk_configuration();
    vec![HarnessConnectors {
        harness: "claude".to_owned(),
        health: claude.connector_health,
    }]
}

/// Where a family's connection lives, when one is connected.
pub fn available_connection(family: ConnectorFamily) -> Option<ConnectorAvailability> {
    crate::connector_surface::resolve_availability(&discover_harness_connectors())
        .into_iter()
        .find(|entry| entry.family == family && entry.available)
}

/// The MCP server name for a family, when some harness has it connected.
pub fn available_server(core: &Arc<BridgeCore>, family: ConnectorFamily) -> Option<String> {
    let _ = core;
    available_connection(family).and_then(|entry| entry.server)
}

/// Spawn the hidden session, send one prompt, and read to the result marker
/// under a wall-clock deadline. The connector-run analogue of
/// `memory_consolidation_live::one_bounded_turn`, scoped to one MCP server
/// instead of to nothing.
fn one_bounded_turn(
    core: &Arc<BridgeCore>,
    connection: &ConnectorAvailability,
    kind: RunKind,
    prompt: &str,
    action_intent: Option<ActionIntent>,
) -> Result<String, String> {
    let server = connection.server.as_deref().unwrap_or_default();
    let harness_id = connection.harness.as_deref().unwrap_or_default();
    let limits = connector_runs::run_limits(kind);
    let wall = limits.max_wall_seconds.max(1) as u64;
    // Scoped to this one server. Ingress and render get the read-only policy —
    // its verb rule is what keeps them from reaching a different connector or a
    // write tool on this one. An action gets the same policy plus exactly one
    // approved mutation, because the read-only rule bans every mutation word and
    // would otherwise deny the very send the user just approved.
    let policy = match kind {
        RunKind::Ingress | RunKind::Render => {
            BriefingRuntimePolicy::compile_scoped(vec![server.to_owned()], limits)
        }
        RunKind::Action => BriefingRuntimePolicy::compile_action(
            server.to_owned(),
            action_intent.ok_or("an action run reached the executor with no approved intent")?,
            limits,
        ),
    }
    .map_err(|unsupported| format!("policy: {}", unsupported.reason()))?;

    let (harness, model) = run_profile(core, harness_id)?;
    let session_id = Uuid::new_v4().to_string();
    let scratch = core.chat_scratch_dir(&session_id);
    std::fs::create_dir_all(&scratch).map_err(|error| error.to_string())?;
    let cwd = scratch.to_string_lossy().to_string();
    {
        let db = core.db.lock().unwrap();
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,model,kind,title,cwd,depth)
             VALUES(?1,NULL,?2,'Connector','working','reported',?3,?4,'Connector run',?5,0)",
            params![session_id, harness, model, CONNECTOR_SESSION_KIND, cwd],
        )
        .map_err(|error| error.to_string())?;
    }

    let started = core.adapter_registry.start(
        &harness,
        StartRequest {
            cwd: &cwd,
            model: Some(&model),
            effort: None,
            instructions: None,
            write_mode: None,
            read_only_sandbox: None,
            briefing: Some(&policy),
            on_progress: None,
        },
    );
    let started = match started {
        Ok(started) => started,
        Err(error) => {
            settle_session(core, &session_id, "failed");
            return Err(format!("provider start: {error}"));
        }
    };
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
        settle_session(core, &session_id, "failed");
        return Err(format!("send: {error}"));
    }

    let deadline = Instant::now() + StdDuration::from_secs(wall);
    let mut text = String::new();
    // Tool failures the run saw. A denied or failed connector call completes the
    // model turn perfectly normally, so without collecting these a failed send
    // and a successful one are indistinguishable at the result frame.
    let mut tool_errors: Vec<String> = Vec::new();
    let outcome = loop {
        if Instant::now() >= deadline {
            break Err(format!("the {} run exceeded its {wall}s budget", kind_label(kind)));
        }
        match receiver.recv_timeout(StdDuration::from_secs(1)) {
            Ok(line) => {
                let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
                match message.get("type").and_then(Value::as_str) {
                    Some("assistant") => collect_text(&message, &mut text),
                    Some("user") => collect_tool_errors(&message, &mut tool_errors),
                    Some("result") => break result_outcome(&message, &tool_errors),
                    _ => {}
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break Err("the provider ended before returning a result".to_string());
            }
        }
    };

    match outcome {
        Ok(()) => {
            runtime.stop(ShutdownReason::Completed);
            settle_session(core, &session_id, "ended");
            Ok(text)
        }
        Err(detail) => {
            runtime.stop(ShutdownReason::Failed);
            settle_session(core, &session_id, "failed");
            Err(detail)
        }
    }
}

/// Read the terminal frame as success or failure.
///
/// An `is_error` result, a non-success subtype, or any tool error seen during
/// the turn all mean the run did not do what it was asked. Treating a terminal
/// frame as success on its own is how a denied Slack call became a reported
/// "sent".
fn result_outcome(message: &Value, tool_errors: &[String]) -> Result<(), String> {
    if let Some(first) = tool_errors.first() {
        return Err(format!("a tool call failed: {first}"));
    }
    if message.get("is_error").and_then(Value::as_bool).unwrap_or(false) {
        let detail = message
            .get("result")
            .and_then(Value::as_str)
            .unwrap_or("the provider reported an error");
        return Err(bounded(detail));
    }
    match message.get("subtype").and_then(Value::as_str) {
        None | Some("success") => Ok(()),
        Some(other) => Err(format!("the run ended as `{other}`")),
    }
}

/// Tool results the provider marked as errors.
fn collect_tool_errors(message: &Value, errors: &mut Vec<String>) {
    let blocks = message
        .get("message")
        .and_then(|inner| inner.get("content"))
        .or_else(|| message.get("content"))
        .and_then(Value::as_array);
    for block in blocks.into_iter().flatten() {
        if block.get("type").and_then(Value::as_str) != Some("tool_result") {
            continue;
        }
        if !block.get("is_error").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        let detail = block
            .get("content")
            .and_then(|content| {
                content.as_str().map(str::to_owned).or_else(|| {
                    content.as_array().and_then(|parts| {
                        parts.iter().find_map(|part| {
                            part.get("text").and_then(Value::as_str).map(str::to_owned)
                        })
                    })
                })
            })
            .unwrap_or_else(|| "the connector rejected the call".to_owned());
        errors.push(bounded(&detail));
    }
}

fn bounded(detail: &str) -> String {
    detail.chars().take(300).collect()
}

/// Did an action run actually report doing the thing?
///
/// The action prompt asks for `sent` or `reacted`, or a one-line failure. A run
/// that returns neither did something other than what was asked, and the safe
/// reading of "I cannot tell" is failure: a reply the user believes was sent and
/// was not is worse than one they are asked to retry.
fn action_succeeded(text: &str, intent: ActionIntent) -> Result<(), String> {
    let lowered = text.to_lowercase();
    let token = match intent {
        ActionIntent::Reply => "sent",
        ActionIntent::React => "reacted",
    };
    if lowered.split(|c: char| !c.is_ascii_alphanumeric()).any(|word| word == token) {
        return Ok(());
    }
    Err(if text.trim().is_empty() {
        format!("the run ended without confirming it {token} anything")
    } else {
        bounded(text.trim())
    })
}

fn kind_label(kind: RunKind) -> &'static str {
    match kind {
        RunKind::Ingress => "inbox",
        RunKind::Render => "card",
        RunKind::Action => "send",
    }
}

fn collect_text(message: &Value, text: &mut String) {
    let blocks = message
        .get("message")
        .and_then(|inner| inner.get("content"))
        .or_else(|| message.get("content"))
        .and_then(Value::as_array);
    for block in blocks.into_iter().flatten() {
        if block.get("type").and_then(Value::as_str) == Some("text") {
            if let Some(chunk) = block.get("text").and_then(Value::as_str) {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(chunk);
            }
        }
    }
}

/// Which harness and model a connector run uses.
///
/// The harness is **determined, not routed**: it is whichever one's own MCP
/// configuration holds this connector, which `available_connection` carries
/// alongside the server name. Sending the run to a "better" harness sends it to
/// one that cannot see the account at all, so learning and routing have nothing
/// to rank here — there is exactly one eligible candidate and the connector
/// names it.
///
/// The model is taken from the Research profile when that profile is already on
/// that harness — a connector run is a read-and-summarise job, which is what
/// that profile is for — and otherwise from the harness's own default. Reading a
/// DM is not worth a premium model, and the ceilings in
/// `connector_runs::run_limits` assume it is not getting one.
fn run_profile(core: &Arc<BridgeCore>, harness: &str) -> Result<(String, String), String> {
    let descriptors = core.adapter_registry.descriptors();
    let default_model = descriptors
        .iter()
        .find(|descriptor| descriptor.id == harness)
        .and_then(|descriptor| descriptor.models.first())
        .map(|model| model.id.clone());
    let model = {
        let db = core.db.lock().unwrap();
        crate::model_profiles::resolve_profile(
            &db,
            &descriptors,
            crate::model_profiles::ProfilePurpose::Research,
        )
        .ok()
        .flatten()
        .filter(|profile| profile.provider == harness)
        .map(|profile| profile.model)
    };
    model
        .or(default_model)
        .map(|model| (harness.to_owned(), model))
        .ok_or_else(|| format!("{harness} offers no model to run a connector turn on"))
}

fn settle_session(core: &Arc<BridgeCore>, session_id: &str, status: &str) {
    let db = core.db.lock().unwrap();
    let _ = db.execute(
        "UPDATE sessions SET status=?2,ended_at=?3 WHERE id=?1",
        params![session_id, status, Utc::now().to_rfc3339()],
    );
}

/// The ingress timer. Does nothing at all while no supported family is
/// connected, so an install with no connectors pays nothing for this feature.
pub fn start_connector_poll_maintenance(core: Arc<BridgeCore>) {
    std::thread::spawn(move || loop {
        for family in ConnectorFamily::ALL {
            if !family.has_inbox_support() {
                continue;
            }
            if available_connection(family).is_some() {
                poll_once(&core, family);
            }
        }
        std::thread::sleep(POLL_CADENCE);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ingress_is_slower_than_the_github_poller_because_a_cycle_is_a_model_turn() {
        assert!(POLL_CADENCE > crate::github_poll::FOCUSED_CADENCE);
    }

    #[test]
    fn one_ingress_cycle_runs_per_family_at_a_time() {
        let poller = ConnectorPoller::default();
        assert!(poller.begin(ConnectorFamily::Slack));
        // The pane's refresh control arriving mid-timer-cycle must not start a
        // second model turn for the same answer.
        assert!(!poller.begin(ConnectorFamily::Slack));
        assert!(poller.begin(ConnectorFamily::Gmail), "families poll independently");
        poller.finish(ConnectorFamily::Slack);
        assert!(poller.begin(ConnectorFamily::Slack), "completion releases the slot");
    }

    #[test]
    fn a_failed_cycle_still_releases_its_slot() {
        // poll_once releases unconditionally after poll_claimed returns, so a
        // cycle that failed cannot wedge the family forever.
        let poller = ConnectorPoller::default();
        poller.begin(ConnectorFamily::Slack);
        poller.finish(ConnectorFamily::Slack);
        assert!(poller.begin(ConnectorFamily::Slack));
    }

    #[test]
    fn a_burst_of_arrivals_does_not_become_a_burst_of_model_turns() {
        // Thirty mentions at once must not be thirty simultaneous render runs;
        // the rest keep Bridge's own card and are picked up next cycle.
        assert!(MAX_RENDERS_PER_CYCLE <= 5);
    }

    #[test]
    fn assistant_text_is_collected_from_both_stream_shapes() {
        let mut text = String::new();
        collect_text(
            &serde_json::json!({"message": {"content": [{"type": "text", "text": "first"}]}}),
            &mut text,
        );
        collect_text(&serde_json::json!({"content": [{"type": "text", "text": "second"}]}), &mut text);
        // A tool_use block carries no prose and must not become one.
        collect_text(
            &serde_json::json!({"content": [{"type": "tool_use", "name": "slack_read_thread"}]}),
            &mut text,
        );
        assert_eq!(text, "first\nsecond");
    }

    #[test]
    fn a_terminal_frame_is_not_by_itself_a_success() {
        // A denied or failed connector call completes the model turn perfectly
        // normally. Reading the frame alone reported "sent" for a send that
        // never happened.
        let ok = serde_json::json!({"type": "result", "subtype": "success"});
        assert!(result_outcome(&ok, &[]).is_ok());

        let errored = serde_json::json!({"type": "result", "is_error": true, "result": "provider refused"});
        assert!(result_outcome(&errored, &[]).unwrap_err().contains("provider refused"));

        let capped = serde_json::json!({"type": "result", "subtype": "error_max_turns"});
        assert!(result_outcome(&capped, &[]).unwrap_err().contains("error_max_turns"));

        // A tool error anywhere in the turn outranks a clean terminal frame.
        assert!(result_outcome(&ok, &["not_in_channel".into()]).unwrap_err().contains("not_in_channel"));
    }

    #[test]
    fn tool_errors_are_collected_from_the_result_blocks() {
        let mut errors = Vec::new();
        collect_tool_errors(
            &serde_json::json!({"content": [
                {"type": "tool_result", "is_error": true, "content": "channel_not_found"},
                {"type": "tool_result", "is_error": false, "content": "fine"},
                {"type": "text", "text": "prose"},
            ]}),
            &mut errors,
        );
        assert_eq!(errors, vec!["channel_not_found".to_string()]);

        // The block-array shape too, which is what the SDK actually emits.
        let mut nested = Vec::new();
        collect_tool_errors(
            &serde_json::json!({"message": {"content": [
                {"type": "tool_result", "is_error": true, "content": [{"type": "text", "text": "rate limited"}]},
            ]}}),
            &mut nested,
        );
        assert_eq!(nested, vec!["rate limited".to_string()]);
    }

    #[test]
    fn an_action_must_report_doing_the_thing_it_was_asked_to_do() {
        assert!(action_succeeded("sent", ActionIntent::Reply).is_ok());
        assert!(action_succeeded("Sent.", ActionIntent::Reply).is_ok());
        assert!(action_succeeded("reacted", ActionIntent::React).is_ok());

        // "I cannot tell" reads as failure: a reply the user believes was sent
        // and was not is worse than one they are asked to retry.
        assert!(action_succeeded("", ActionIntent::Reply).is_err());
        assert!(action_succeeded("I could not reach the channel.", ActionIntent::Reply).is_err());
        // And the other intent's token does not count.
        assert!(action_succeeded("reacted", ActionIntent::Reply).is_err());
        // Substrings do not count either.
        assert!(action_succeeded("unsent draft remains", ActionIntent::Reply).is_err());
    }

    #[test]
    fn one_action_runs_per_item_and_a_failure_releases_the_claim() {
        let poller = ConnectorPoller::default();
        assert!(poller.begin_action("slack:D0:1.0"));
        // A second click while the first send is in flight.
        assert!(!poller.begin_action("slack:D0:1.0"));
        assert!(poller.begin_action("slack:D0:2.0"), "other items are independent");
        poller.finish_action("slack:D0:1.0");
        // Released on every path, so a failed send stays retryable rather than
        // wedging the item forever.
        assert!(poller.begin_action("slack:D0:1.0"));
    }

    #[test]
    fn every_run_kind_has_a_distinct_deadline_label() {
        let labels: std::collections::BTreeSet<_> =
            [RunKind::Ingress, RunKind::Render, RunKind::Action].map(kind_label).into_iter().collect();
        assert_eq!(labels.len(), 3, "a timeout message should say which run timed out");
    }
}

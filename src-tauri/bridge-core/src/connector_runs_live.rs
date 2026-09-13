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
use std::sync::{mpsc, Arc};
use std::time::{Duration as StdDuration, Instant};

use chrono::Utc;
use rusqlite::params;
use serde_json::Value;
use uuid::Uuid;

use crate::adapters::{ShutdownReason, StartRequest};
use crate::briefing_policy::BriefingRuntimePolicy;
use crate::connector_inbox::{self, Resolution};
use crate::connector_runs::{
    self, AuthorizedAction, ConnectorAction, RunKind, CARD_FENCE, INGRESS_FENCE,
};
use crate::connector_surface::{ConnectorCard, InboxItem};
use crate::events::CoreEvent;
use crate::runtime::BridgeCore;
use crate::work_connectors::ConnectorFamily;

/// Hidden session kind. No surface lists these.
pub const CONNECTOR_SESSION_KIND: &str = "connector";

/// How often ingress runs while the window has focus, and while it does not.
///
/// Slower than the GitHub poller's 15s on purpose: each cycle here is a model
/// turn, not a `gh` call. A minute of latency on a DM is the price of a surface
/// that costs almost nothing to leave on, and push ingress is the fix if it ever
/// stops being an acceptable trade.
pub const FOCUSED_CADENCE: StdDuration = StdDuration::from_secs(30);
pub const UNFOCUSED_CADENCE: StdDuration = StdDuration::from_secs(180);

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
    let now = Utc::now().to_rfc3339();
    let Some(server) = available_server(core, family) else {
        let db = core.db.lock().unwrap();
        let _ = connector_inbox::record_poll_failure(
            &db,
            family,
            &now,
            &format!("{} is not connected in this harness", family.display_name()),
        );
        return 0;
    };

    let output = match one_bounded_turn(
        core,
        &server,
        RunKind::Ingress,
        &connector_runs::ingress_prompt(family),
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

    for item in fresh.iter().take(MAX_RENDERS_PER_CYCLE) {
        render_item(core, &server, item);
    }
    fresh.len()
}

/// Render exactly one item and attach the result.
///
/// This is the whole "render only what a notification needs" rule in one
/// function: it is called per arrival, it is given one item, and there is no
/// batch or refresh variant of it anywhere. Re-opening the pane re-reads the
/// stored card; it does not come back here.
pub fn render_item(core: &Arc<BridgeCore>, server: &str, item: &InboxItem) {
    let (card, rejection) = match one_bounded_turn(
        core,
        server,
        RunKind::Render,
        &connector_runs::render_prompt(item),
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
    let server = available_server(core, item.family)
        .ok_or_else(|| format!("{} is not connected in this harness", item.family.display_name()))?;

    // Claim the item *before* the run. A send that succeeds and then fails to
    // record would let the next click send again; claiming first means the worst
    // case is a resolved item whose send failed, which the user can see and redo.
    let claimed = {
        let db = core.db.lock().unwrap();
        connector_inbox::resolve(
            &db,
            &item.key(),
            match authorized.action() {
                ConnectorAction::Reply { .. } => Resolution::Replied,
                ConnectorAction::React { .. } => Resolution::Reacted,
            },
            &Utc::now().to_rfc3339(),
        )
        .unwrap_or(false)
    };
    if !claimed {
        return Err("this message has already been dealt with".into());
    }

    let outcome = one_bounded_turn(core, &server, RunKind::Action, &authorized.prompt());
    core.events.publish(CoreEvent::ConnectorItemResolved {
        family: item.family.as_str().into(),
        item_key: item.key(),
        succeeded: outcome.is_ok(),
    });
    core.events.publish(CoreEvent::ConnectorInboxChanged {
        family: item.family.as_str().into(),
    });
    outcome.map(|_| ())
}

/// The MCP server name for a family, when the harness has it connected.
pub fn available_server(core: &Arc<BridgeCore>, family: ConnectorFamily) -> Option<String> {
    let _ = core;
    let configuration = crate::marketplace::claude_sdk_configuration();
    crate::connector_surface::resolve_availability(&configuration.connector_health)
        .into_iter()
        .find(|entry| entry.family == family && entry.available)
        .and_then(|entry| entry.server)
}

/// Spawn the hidden session, send one prompt, and read to the result marker
/// under a wall-clock deadline. The connector-run analogue of
/// `memory_consolidation_live::one_bounded_turn`, scoped to one MCP server
/// instead of to nothing.
fn one_bounded_turn(
    core: &Arc<BridgeCore>,
    server: &str,
    kind: RunKind,
    prompt: &str,
) -> Result<String, String> {
    let limits = connector_runs::run_limits(kind);
    let wall = limits.max_wall_seconds.max(1) as u64;
    // Scoped to this one server: the policy's read-verb rule is what keeps an
    // ingress run from reaching a different connector, and what keeps a render
    // run from reaching a write tool on this one.
    let policy = BriefingRuntimePolicy::compile_scoped(vec![server.to_owned()], limits)
        .map_err(|unsupported| format!("policy: {}", unsupported.reason()))?;

    let (harness, model) = run_profile(core)?;
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
    let outcome = loop {
        if Instant::now() >= deadline {
            break Err(format!("the {} run exceeded its {wall}s budget", kind_label(kind)));
        }
        match receiver.recv_timeout(StdDuration::from_secs(1)) {
            Ok(line) => {
                let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
                match message.get("type").and_then(Value::as_str) {
                    Some("assistant") => collect_text(&message, &mut text),
                    Some("result") => break Ok(()),
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
/// The harness is **pinned**, not routed. The premise of the whole feature is
/// that the connector lives in one harness's own MCP configuration, so sending
/// this run to a "better" harness sends it to one that cannot see the account at
/// all. Learning and routing rank eligible candidates; here there is exactly one.
///
/// The model is taken from the Research profile when that profile is already on
/// this harness — a connector run is a read-and-summarise job, which is what
/// that profile is for — and otherwise from the adapter's own default. Reading a
/// DM is not worth a premium model, and the ceilings in
/// `connector_runs::run_limits` assume it is not getting one.
fn run_profile(core: &Arc<BridgeCore>) -> Result<(String, String), String> {
    const HARNESS: &str = "claude";
    let descriptors = core.adapter_registry.descriptors();
    let model = {
        let db = core.db.lock().unwrap();
        crate::model_profiles::resolve_profile(
            &db,
            &descriptors,
            crate::model_profiles::ProfilePurpose::Research,
        )
        .ok()
        .flatten()
        .filter(|profile| profile.provider == HARNESS)
        .map(|profile| profile.model)
    };
    Ok((
        HARNESS.to_owned(),
        model.unwrap_or_else(|| crate::claude_adapter::DEFAULT_MODEL.to_owned()),
    ))
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
            if available_server(&core, family).is_some() {
                poll_once(&core, family);
            }
        }
        std::thread::sleep(FOCUSED_CADENCE);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ingress_is_slower_than_the_github_poller_because_a_cycle_is_a_model_turn() {
        assert!(FOCUSED_CADENCE > crate::github_poll::FOCUSED_CADENCE);
        assert!(UNFOCUSED_CADENCE > FOCUSED_CADENCE);
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
    fn every_run_kind_has_a_distinct_deadline_label() {
        let labels: std::collections::BTreeSet<_> =
            [RunKind::Ingress, RunKind::Render, RunKind::Action].map(kind_label).into_iter().collect();
        assert_eq!(labels.len(), 3, "a timeout message should say which run timed out");
    }
}

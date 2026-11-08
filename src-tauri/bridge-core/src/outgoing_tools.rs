//! The Outgoing tool, core side: `outgoing_draft`.
//!
//! It rides the chat-tools route the notes and memory tools use (same
//! per-chat token, same `bridge` MCP server), so every harness that loads the
//! `bridge` server gets it. A draft is all an agent can make: it cannot copy,
//! edit, approve or send one, and nothing here reaches the network. The
//! person does the rest from the card the draft becomes in the transcript.

use bridge_protocol::messages::OutgoingMessage;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::notes::tool::ToolError;
use crate::{memory_extraction, outgoing};

pub const DRAFT: &str = "outgoing_draft";
pub const WRITE_TOOLS: [&str; 1] = [DRAFT];

/// What the agent reads back. It tells the agent the person takes it from
/// here, so it does not paste the same text into its reply.
pub const DRAFT_NOTE: &str = "Shown to the person as a draft card they can edit, copy and send themselves. You cannot send it. Do not repeat the message in your reply; say in a sentence what you drafted and for whom.";

/// Exactly the three fields the tool offers. A `state`, `revision` or
/// anything else makes the call invalid rather than honored.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftArgs {
    to: Value,
    body: String,
    reason: String,
    #[serde(default)]
    attachments: Vec<String>,
}

pub struct Outcome {
    pub result: Value,
    pub item: OutgoingMessage,
}

fn bad(message: impl Into<String>) -> ToolError {
    ToolError { status: 400, message: message.into() }
}

fn internal(error: crate::BridgeError) -> ToolError {
    ToolError { status: 500, message: error.to_string() }
}

pub fn run(db: &Connection, session_id: &str, can_write: bool, tool: &str, args: &Value) -> Result<Outcome, ToolError> {
    if tool != DRAFT {
        return Err(ToolError { status: 404, message: format!("unknown outgoing tool {tool}") });
    }
    if !can_write {
        return Err(ToolError { status: 403, message: "Only a top-level chat can draft messages.".into() });
    }
    let args: DraftArgs = serde_json::from_value(args.clone())
        .map_err(|error| bad(format!("outgoing_draft takes {{to, body, reason}} and optionally attachments: {error}")))?;
    let destination = outgoing::parse_destination(&args.to).map_err(bad)?;
    let reason = args.reason.trim();
    if reason.is_empty() || reason.chars().count() > outgoing::MAX_REASON_CHARS {
        return Err(bad(format!(
            "reason must say in at most {} characters why the person wants this message",
            outgoing::MAX_REASON_CHARS
        )));
    }
    outgoing::check_body(&args.body).map_err(bad)?;
    if memory_extraction::contains_secret_material(&args.body) {
        return Err(bad("The message holds what looks like a credential. Leave it out; nothing was drafted."));
    }
    if outgoing::count_for_session(db, session_id).map_err(internal)? >= outgoing::MAX_DRAFTS_PER_CHAT {
        return Err(ToolError {
            status: 429,
            message: format!("This chat already has {} drafts; ask the person before drafting more.", outgoing::MAX_DRAFTS_PER_CHAT),
        });
    }
    let now = chrono::Utc::now().to_rfc3339();
    let item = outgoing::create_draft(db, session_id, destination, &args.body, reason, &args.attachments, &now).map_err(|error| match error {
        crate::BridgeError::Invalid(message) => bad(message),
        other => internal(other),
    })?;
    Ok(Outcome {
        result: json!({
            "id": item.id,
            "state": "draft",
            "destination": item.label,
            "rendered": item.rendered,
            "lint": item.lint,
            "attachments": item.attachments.iter().map(|attachment| attachment.name.clone()).collect::<Vec<_>>(),
            "note": DRAFT_NOTE,
        }),
        item,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(body: &str) -> Value {
        json!({"to": {"slack": {"channel": "#dev"}}, "body": body, "reason": "tell the team the deploy finished"})
    }

    #[test]
    fn a_draft_is_stored_and_reported_back_with_its_fixes() {
        let db = outgoing::tests::store();
        let outcome = run(&db, "chat", true, DRAFT, &args("Deploy is done \u{2014} all green")).unwrap();
        assert_eq!(outcome.result["rendered"], "Deploy is done, all green");
        assert_eq!(outcome.result["destination"], "Slack \u{b7} #dev");
        assert_eq!(outcome.result["lint"][0]["rule"], "no_em_dash");
        assert_eq!(outcome.result["lint"][0]["fixed"], true);
        assert_eq!(outgoing::get(&db, outcome.result["id"].as_str().unwrap()).unwrap().unwrap(), outcome.item);
    }

    #[test]
    fn unknown_fields_are_refused() {
        let db = outgoing::tests::store();
        let mut call = args("hi");
        call["state"] = json!("sent");
        let error = run(&db, "chat", true, DRAFT, &call).err().unwrap();
        assert_eq!(error.status, 400);
        assert!(error.message.contains("{to, body, reason}"), "{}", error.message);
        let error = run(&db, "chat", true, DRAFT, &json!({"to": {"email": "a"}, "body": "hi", "reason": "r"})).err().unwrap();
        assert!(error.message.contains("to must be exactly one of"), "{}", error.message);
        assert_eq!(outgoing::count_for_session(&db, "chat").unwrap(), 0);
    }

    #[test]
    fn secret_material_is_refused_and_not_stored() {
        let db = outgoing::tests::store();
        let error = run(&db, "chat", true, DRAFT, &args("use sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGH")).err().unwrap();
        assert_eq!(error.status, 400);
        assert_eq!(outgoing::count_for_session(&db, "chat").unwrap(), 0);
    }

    #[test]
    fn read_only_access_cannot_draft() {
        let db = outgoing::tests::store();
        assert_eq!(run(&db, "worker", false, DRAFT, &args("hi")).err().unwrap().status, 403);
        assert_eq!(run(&db, "chat", true, "outgoing_send", &args("hi")).err().unwrap().status, 404);
    }

    #[test]
    fn per_chat_cap_is_enforced() {
        let db = outgoing::tests::store();
        for n in 0..outgoing::MAX_DRAFTS_PER_CHAT {
            run(&db, "chat", true, DRAFT, &args(&format!("message {n}"))).unwrap();
        }
        assert_eq!(run(&db, "chat", true, DRAFT, &args("one more")).err().unwrap().status, 429);
        assert!(run(&db, "other", true, DRAFT, &args("fine")).is_ok(), "the cap is per chat");
    }

    #[test]
    fn images_ride_along_when_the_agent_attaches_them() {
        let db = outgoing::tests::store();
        let dir = tempfile::tempdir().unwrap();
        let png = outgoing::attachments::tests::write(dir.path(), "shot.png", outgoing::attachments::tests::PNG);
        let mut call = args("Screenshot of the bug");
        call["attachments"] = json!([png]);
        let outcome = run(&db, "chat", true, DRAFT, &call).unwrap();
        assert_eq!(outcome.result["attachments"], json!(["shot.png"]));
        call["attachments"] = json!(["/etc/hosts"]);
        let error = run(&db, "chat", true, DRAFT, &call).err().unwrap();
        assert_eq!(error.status, 400, "a non-image is the agent's mistake, not a server error");
        assert!(error.message.contains("not a PNG"), "{}", error.message);
    }

    #[test]
    fn empty_body_or_reason_is_refused() {
        let db = outgoing::tests::store();
        assert_eq!(run(&db, "chat", true, DRAFT, &args("   ")).err().unwrap().status, 400);
        let call = json!({"to": {"clipboard": "plain"}, "body": "hi", "reason": " "});
        assert_eq!(run(&db, "chat", true, DRAFT, &call).err().unwrap().status, 400);
    }
}

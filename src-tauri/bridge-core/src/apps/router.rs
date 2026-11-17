//! The personal-app router: one door for every agent.
//!
//! A chat on any supported agent drafts and previews apps through Bridge's own
//! MCP tools. Each app is bound to a connection, `{harness, account, server}`,
//! and every MCP request for it comes here. The router decides authority,
//! then picks a path:
//! - the chat's own live runtime, when the chat *is* that connection (same
//!   agent and account), which also keeps that chat's project MCPs;
//! - otherwise a warm connection runtime for exactly that connection
//!   ([`super::runtimes`]).
//!
//! Adapters only carry requests. A call never reaches another account,
//! because both paths are chosen by the full connection identity.

use super::{control, runtimes};
use crate::{delegation::WriteMode, runtime::BridgeCore, BridgeError};
use rusqlite::{Connection as Db, OptionalExtension};
use serde_json::{json, Value};
use std::sync::Arc;

/// Agents whose top-level chats can draft and open personal apps.
pub const CHAT_HARNESSES: &[&str] = &["claude", "codex", "opencode", "cursor"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Inventory,
    Read,
    Write,
}

/// How an agent can carry one operation for a connection it owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// Bridge calls the agent's own MCP client with no model turn.
    Direct,
    /// Not available from this agent yet; the reason names what to do.
    Unsupported,
}

pub fn capability(harness: &str, _op: Op) -> Capability {
    match harness {
        "claude" | "codex" => Capability::Direct,
        _ => Capability::Unsupported,
    }
}

pub fn agent_label(harness: &str) -> &str {
    match harness {
        "claude" => "Claude Code",
        "codex" => "Codex",
        "opencode" => "OpenCode",
        "cursor" => "Cursor",
        "grok" => "Grok",
        "pi" => "Pi",
        other => other,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    pub harness: String,
    pub account: Option<String>,
    /// The project folder the server resolves in, or `None` for user-wide.
    /// A project can configure a server under a user-wide server's name, so
    /// the same name on the same account can be a different server here.
    pub cwd: Option<String>,
    pub server: String,
}

fn invalid(text: impl Into<String>) -> BridgeError {
    BridgeError::Invalid(text.into())
}

/// The chat's own agent, account and project folder.
struct ChatIdentity {
    harness: String,
    account: Option<String>,
    cwd: Option<String>,
}

fn chat_identity(core: &Arc<BridgeCore>, chat: &str) -> Result<ChatIdentity, BridgeError> {
    let (harness, account, cwd) = core
        .try_db()?
        .query_row("SELECT harness, account, cwd FROM sessions WHERE id=?1", [chat], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<String>>(2)?))
        })
        .optional()?
        .ok_or_else(|| invalid("This chat no longer exists"))?;
    let scratch_root = core.chat_scratch_dir(chat).parent().map(std::path::Path::to_path_buf);
    Ok(ChatIdentity { harness, account, cwd: project_cwd(cwd.as_deref(), scratch_root.as_deref()) })
}

/// A chat's folder when it is a real project; a chat's own scratch folder
/// resolves exactly as the user-wide configuration does.
fn project_cwd(cwd: Option<&str>, scratch_root: Option<&std::path::Path>) -> Option<String> {
    let cwd = cwd.filter(|cwd| !cwd.is_empty())?;
    let inside_scratch = scratch_root.is_some_and(|root| std::path::Path::new(cwd).starts_with(root));
    (!inside_scratch).then(|| cwd.to_owned())
}

/// A connector read is allowed where the chat's own access mode would let its
/// model read: Full yes, Ask (Shared) and the restricted modes no. A chat with
/// no stored mode launches as Full, so it reads as Full.
pub fn may_read(db: &Db, chat: &str) -> Result<(), BridgeError> {
    match crate::chat_defaults::root_write_mode(db, chat)? {
        None | Some(WriteMode::Full) => Ok(()),
        Some(_) => Err(invalid(
            "This chat's access mode does not allow connector reads; switch the chat to Full access to preview live reads",
        )),
    }
}

/// Send one app-control request for `connection`, on behalf of `chat`.
fn send(core: &Arc<BridgeCore>, chat: &str, connection: &Connection, request: &Value) -> Result<Value, BridgeError> {
    if capability(&connection.harness, Op::Inventory) != Capability::Direct {
        return Err(invalid(format!(
            "{} cannot run personal apps directly yet; connect {} in Claude Code or Codex",
            agent_label(&connection.harness),
            connection.server
        )));
    }
    let identity = chat_identity(core, chat)?;
    // The chat's own process only when it resolves servers exactly as the
    // connection did: same agent, same account, same project folder.
    let live = identity.harness == connection.harness
        && identity.account == connection.account
        && identity.cwd == connection.cwd
        && core
            .adapters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(chat);
    if live {
        return control::request(core, chat, request);
    }
    let key = runtimes::Key {
        harness: connection.harness.clone(),
        account: connection.account.clone(),
        cwd: connection.cwd.clone(),
        server: connection.server.clone(),
    };
    runtimes::request_with(&key, request, &runtimes::launcher(core))
}

pub fn inventory(core: &Arc<BridgeCore>, chat: &str, connection: &Connection) -> Result<Value, BridgeError> {
    send(core, chat, connection, &json!({"kind": "inventory", "server": connection.server}))
}

/// One allowlisted read. The caller has already checked the tool against the
/// app's stored, hash-verified allowlist; the chat's access mode is checked
/// here so no path can skip it.
pub fn read(core: &Arc<BridgeCore>, chat: &str, connection: &Connection, tool: &str, arguments: &Value) -> Result<Value, BridgeError> {
    may_read(&*core.try_db()?, chat)?;
    read_with(connection, tool, arguments, |request| send(core, chat, connection, request))
}

/// The read itself, after the server's *current* inventory says the tool is
/// still a safe read. A draft can outlive a server update that turns a tool
/// destructive, and Codex runs `mcpServer/tool/call` with no approval of its
/// own, so the annotations are checked on the same path before every call.
pub fn read_with(
    connection: &Connection,
    tool: &str,
    arguments: &Value,
    mut send: impl FnMut(&Value) -> Result<Value, BridgeError>,
) -> Result<Value, BridgeError> {
    if !crate::briefing_policy::is_read_tool_name(tool) {
        return Err(invalid("This tool is not a read"));
    }
    let inventory = send(&json!({"kind": "inventory", "server": connection.server}))?;
    if inventory["status"] != "connected" {
        return Err(invalid(format!("{} is not connected; reconnect it and try again", connection.server)));
    }
    let current = inventory["tools"]
        .as_array()
        .and_then(|tools| tools.iter().find(|candidate| candidate["name"].as_str() == Some(tool)))
        .ok_or_else(|| invalid(format!("{} no longer offers {tool}", connection.server)))?;
    if current["readOnly"] == false || current["destructive"] == true {
        return Err(invalid(format!("{tool} is no longer marked as a safe read, so it was not called")));
    }
    send(&json!({"kind": "read", "server": connection.server, "tool": tool, "arguments": arguments}))
}

/// One approved write. Callers reach this only with a claimed, single-use
/// approval of these exact arguments (see [`super::open`]); nothing here asks
/// again. Claude sends it from a one-shot exact-action launch, Codex with a
/// direct call on the connection's runtime.
pub fn write(core: &Arc<BridgeCore>, chat: &str, connection: &Connection, tool: &str, arguments: &Value) -> Result<Value, BridgeError> {
    match connection.harness.as_str() {
        "claude" => super::write::claude(core, &key_for(connection), tool, arguments),
        "codex" => send(core, chat, connection, &json!({"kind": "write", "server": connection.server, "tool": tool, "arguments": arguments})),
        other => Err(invalid(format!(
            "{} cannot run personal apps directly yet; connect {} in Claude Code or Codex",
            agent_label(other),
            connection.server
        ))),
    }
}

fn key_for(connection: &Connection) -> runtimes::Key {
    runtimes::Key {
        harness: connection.harness.clone(),
        account: connection.account.clone(),
        cwd: connection.cwd.clone(),
        server: connection.server.clone(),
    }
}

/// The connections to try for a server a chat names, in order: the chat's own
/// agent and account when it can run apps directly, then the person's own
/// Claude Code and Codex logins.
pub fn candidates(chat_harness: &str, chat_account: Option<&str>, chat_cwd: Option<&str>, server: &str) -> Vec<Connection> {
    let mut list = Vec::new();
    if capability(chat_harness, Op::Inventory) == Capability::Direct {
        list.push(Connection {
            harness: chat_harness.into(),
            account: chat_account.map(str::to_owned),
            cwd: chat_cwd.map(str::to_owned),
            server: server.into(),
        });
    }
    // Other agents are asked user-wide: a project's servers belong to the
    // agent the person opened that project in.
    for harness in ["claude", "codex"] {
        let candidate = Connection { harness: harness.into(), account: None, cwd: None, server: server.into() };
        if !list.contains(&candidate) {
            list.push(candidate);
        }
    }
    list
}

/// The first candidate with `server` connected, and its inventory.
pub fn choose(core: &Arc<BridgeCore>, chat: &str, server: &str) -> Result<(Connection, Value), BridgeError> {
    let identity = chat_identity(core, chat)?;
    choose_with(
        &candidates(&identity.harness, identity.account.as_deref(), identity.cwd.as_deref(), server),
        |connection| {
            let found = inventory(core, chat, connection);
            // A runtime started only to look for a server that is not there
            // is stopped now, not kept warm for ten minutes.
            if !found.as_ref().is_ok_and(|inventory| inventory["status"] == "connected") {
                runtimes::retire(&runtimes::Key {
                    harness: connection.harness.clone(),
                    account: connection.account.clone(),
                    cwd: connection.cwd.clone(),
                    server: connection.server.clone(),
                });
            }
            found
        },
    )
}

pub fn choose_with(
    candidates: &[Connection],
    mut inventory: impl FnMut(&Connection) -> Result<Value, BridgeError>,
) -> Result<(Connection, Value), BridgeError> {
    let server = candidates.first().map(|c| c.server.clone()).unwrap_or_default();
    for candidate in candidates {
        if let Ok(found) = inventory(candidate) {
            if found["status"] == "connected" {
                return Ok((candidate.clone(), found));
            }
        }
    }
    Err(invalid(format!(
        "No agent that can run personal apps has {server} connected. Connect it in Claude Code or Codex, then ask again"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Arc<BridgeCore>) {
        let dir = tempfile::tempdir().unwrap();
        let core = Arc::new(BridgeCore::for_tests(dir.path()));
        (dir, core)
    }

    #[test]
    fn capabilities_and_labels() {
        for harness in ["claude", "codex"] {
            for op in [Op::Inventory, Op::Read, Op::Write] {
                assert_eq!(capability(harness, op), Capability::Direct);
            }
        }
        for harness in ["opencode", "cursor", "grok", "pi"] {
            assert_eq!(capability(harness, Op::Read), Capability::Unsupported);
        }
        assert_eq!(agent_label("codex"), "Codex");
        assert_eq!(agent_label("cursor"), "Cursor");
    }

    #[test]
    fn candidates_put_the_chat_first_and_never_borrow_its_account() {
        let list = candidates("codex", Some("work"), Some("/repo"), "slack");
        assert_eq!(list[0], Connection { harness: "codex".into(), account: Some("work".into()), cwd: Some("/repo".into()), server: "slack".into() });
        assert_eq!(list[1].harness, "claude");
        assert_eq!(list[2], Connection { harness: "codex".into(), account: None, cwd: None, server: "slack".into() });
        assert!(list[1..].iter().all(|c| c.cwd.is_none()), "other agents are asked user-wide");
        let list = candidates("cursor", Some("work"), Some("/repo"), "slack");
        assert_eq!(list.iter().map(|c| c.harness.as_str()).collect::<Vec<_>>(), ["claude", "codex"]);
        assert!(list.iter().all(|c| c.account.is_none()), "an OpenCode or Cursor account is not a Claude or Codex one");
        assert_eq!(candidates("claude", None, None, "slack").len(), 2, "no duplicate default");
        assert_eq!(candidates("claude", None, Some("/repo"), "slack").len(), 3, "a project connection is not the user-wide one");
    }

    #[test]
    fn a_chat_scratch_folder_is_user_wide_and_a_project_is_not() {
        let root = std::path::Path::new("/data/chats");
        assert_eq!(project_cwd(Some("/data/chats/abc"), Some(root)), None);
        assert_eq!(project_cwd(Some(""), Some(root)), None);
        assert_eq!(project_cwd(None, Some(root)), None);
        assert_eq!(project_cwd(Some("/Users/me/repo"), Some(root)), Some("/Users/me/repo".into()));
    }

    #[test]
    fn the_live_chat_is_used_only_when_its_project_matches() {
        let (_dir, core) = fixture();
        core.db.lock().unwrap().execute_batch("INSERT INTO sessions(id,harness,label,status,cwd) VALUES('chat','codex','Chat','ready','/repo/a')").unwrap();
        let identity = chat_identity(&core, "chat").unwrap();
        assert_eq!(identity.cwd.as_deref(), Some("/repo/a"));
        // With no live adapter registered both fall through to a warm runtime
        // keyed by the folder; the identity is what decides, shown here.
        let same = Connection { harness: "codex".into(), account: None, cwd: Some("/repo/a".into()), server: "review".into() };
        let user_wide = Connection { cwd: None, ..same.clone() };
        assert!(identity.harness == same.harness && identity.account == same.account && identity.cwd == same.cwd);
        assert!(identity.cwd != user_wide.cwd, "a user-wide connection never rides a project chat's process");
    }

    #[test]
    fn choose_takes_the_first_connected_and_refuses_when_none() {
        let list = candidates("cursor", None, None, "slack");
        let (chosen, _) = choose_with(&list, |c| Ok(if c.harness == "codex" { json!({"status":"connected","name":"slack"}) } else { json!({"status":"failed"}) })).unwrap();
        assert_eq!(chosen.harness, "codex");
        let error = choose_with(&list, |_| Err(invalid("down"))).unwrap_err().to_string();
        assert!(error.contains("Connect it in Claude Code or Codex"), "{error}");
    }

    #[test]
    fn every_read_rechecks_the_current_annotations_first() {
        let connection = Connection { harness: "codex".into(), account: None, cwd: None, server: "review".into() };
        let run = |tools: Value, status: &str| {
            let mut sent = Vec::new();
            let result = read_with(&connection, "get_report", &json!({"id": 1}), |request| {
                sent.push(request["kind"].as_str().unwrap_or_default().to_owned());
                Ok(if request["kind"] == "inventory" { json!({"status": status, "tools": tools}) } else { json!({"content": []}) })
            });
            (result, sent)
        };
        let (ok, sent) = run(json!([{"name":"get_report","readOnly":true,"destructive":null}]), "connected");
        assert!(ok.is_ok());
        assert_eq!(sent, ["inventory", "read"]);
        for (tools, status) in [
            (json!([{"name":"get_report","readOnly":false,"destructive":null}]), "connected"),
            (json!([{"name":"get_report","readOnly":null,"destructive":true}]), "connected"),
            (json!([{"name":"other_get","readOnly":true}]), "connected"),
            (json!([{"name":"get_report","readOnly":true}]), "failed"),
        ] {
            let (result, sent) = run(tools.clone(), status);
            assert!(result.is_err(), "{tools} {status}");
            assert_eq!(sent, ["inventory"], "the tool is never called: {tools} {status}");
        }
        let (unannotated, _) = run(json!([{"name":"get_report","readOnly":null,"destructive":null}]), "connected");
        assert!(unannotated.is_ok(), "an unannotated read verb stays a read, as at draft time");
        assert!(read_with(&connection, "delete_report", &json!({}), |_| panic!("never sent")).is_err());
    }

    #[test]
    fn reads_follow_the_chat_access_mode() {
        let (_dir, core) = fixture();
        let db = core.db.lock().unwrap();
        db.execute_batch("INSERT INTO sessions(id,harness,label,status) VALUES('chat','codex','Chat','ready')").unwrap();
        assert!(may_read(&db, "chat").is_ok(), "no stored mode launches as Full");
    }

    #[test]
    fn unsupported_connections_refuse_with_the_agent_name() {
        let (_dir, core) = fixture();
        core.db.lock().unwrap().execute_batch("INSERT INTO sessions(id,harness,label,status) VALUES('chat','cursor','Chat','ready')").unwrap();
        let error = inventory(&core, "chat", &Connection { harness: "opencode".into(), account: None, cwd: None, server: "railway".into() }).unwrap_err().to_string();
        assert!(error.contains("OpenCode") && error.contains("Claude Code or Codex"), "{error}");
    }
}

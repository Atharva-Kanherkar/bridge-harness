//! Opening a locked personal app in a chat, and every call its view makes.
//!
//! `apps_open` binds one chat to one exact locked version (its content hash)
//! and the model's prefill. The card reloads that binding, never the model's
//! tool input. Each call from the view comes back here, where the binding,
//! the hash and the allowlist are rechecked:
//! - a read goes through the router (access mode, current annotations);
//! - a write never runs on its first call. Its arguments are canonicalized
//!   once and held behind a single-use token; the card shows exactly that
//!   canonical JSON, and approving sends exactly it, parsed back;
//! - anything else is refused.
//!
//! Every call leaves one ledger row, never with arguments or results.

use super::{
    personal::{self, LockedVersion},
    router::{self, Connection},
};
use crate::{runtime::BridgeCore, BridgeError};
use bridge_protocol::messages::{CallPersonalAppToolParams, GetPersonalAppOpenParams, PersonalAppCallResult, PersonalAppOpen};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection as Db, OptionalExtension, Transaction};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Instant;

/// How long an approval sheet's token stays good.
pub const APPROVAL_TTL_SECS: i64 = 600;
/// How many locked apps the `apps_open` description names.
pub const LISTED_APPS: usize = 20;
const MAX_PREFILL_BYTES: usize = 32 * 1024;
const MAX_ARGUMENT_BYTES: usize = 64 * 1024;

fn invalid(text: impl Into<String>) -> BridgeError {
    BridgeError::Invalid(text.into())
}

pub fn install_store(tx: &Transaction<'_>) -> Result<(), BridgeError> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS personal_app_opens(
            open_id TEXT PRIMARY KEY, session_id TEXT NOT NULL, app_id TEXT NOT NULL,
            version INTEGER NOT NULL, content_hash TEXT NOT NULL, prefill TEXT, created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS personal_app_opens_session ON personal_app_opens(session_id);
        CREATE TABLE IF NOT EXISTS personal_app_write_approvals(
            token TEXT PRIMARY KEY, open_id TEXT NOT NULL UNIQUE, session_id TEXT NOT NULL,
            tool TEXT NOT NULL, arguments TEXT NOT NULL, digest TEXT NOT NULL, created_at TEXT NOT NULL
        );",
    )?;
    Ok(())
}

/// The locked apps a chat's `apps_open` description names: id, name, intent.
pub fn listing(db: &Db) -> Result<Vec<Value>, BridgeError> {
    Ok(personal::list(db)?
        .into_iter()
        .take(LISTED_APPS)
        .map(|app| json!({"id": app.id, "name": app.name, "intent": app.intent}))
        .collect())
}

fn unknown_app(db: &Db, app_id: &str) -> BridgeError {
    let apps = listing(db).unwrap_or_default();
    let names: Vec<String> = apps
        .iter()
        .map(|app| format!("{} (app_id: {})", app["name"].as_str().unwrap_or_default(), app["id"].as_str().unwrap_or_default()))
        .collect();
    if names.is_empty() {
        invalid(format!("There is no personal app \"{app_id}\", and the person has no locked apps yet"))
    } else {
        invalid(format!("There is no personal app \"{app_id}\". The person's apps: {}", names.join("; ")))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    app_id: String,
    #[serde(default)]
    prefill: Option<Value>,
}

fn connection(manifest: &personal::Manifest) -> Connection {
    Connection {
        harness: manifest.harness.clone(),
        account: manifest.account.clone(),
        cwd: manifest.cwd.clone(),
        server: manifest.server.clone(),
    }
}

/// A version whose stored content still hashes to its recorded hash.
fn verified(version: LockedVersion) -> Result<LockedVersion, BridgeError> {
    personal::validate(&version.manifest).map_err(invalid)?;
    if personal::content_hash(&version.manifest) != version.content_hash {
        return Err(invalid("This app's stored content no longer matches its lock; it will not run"));
    }
    Ok(version)
}

/// `bridge.apps_open`: bind this chat to the app's latest locked version.
pub fn create(core: &Arc<BridgeCore>, session: &str, args: Value) -> Result<Value, BridgeError> {
    if !super::draft::eligible(&*core.try_db()?, session)? {
        return Err(invalid("Personal apps need a top-level chat in Claude Code, Codex, OpenCode or Cursor"));
    }
    let input: Input = serde_json::from_value(args).map_err(|_| invalid("Invalid apps_open arguments"))?;
    personal::validate_id(&input.app_id).map_err(invalid)?;
    if input.prefill.as_ref().is_some_and(|prefill| !prefill.is_object() || prefill.to_string().len() > MAX_PREFILL_BYTES) {
        return Err(invalid("prefill must be an object within 32 KiB"));
    }
    let version = {
        let db = core.try_db()?;
        let found = personal::latest_locked(&db, &input.app_id)?;
        found.ok_or_else(|| unknown_app(&db, &input.app_id))?
    };
    let version = verified(version)?;
    let connection = connection(&version.manifest);
    let inventory = router::inventory(core, session, &connection)?;
    if inventory["status"] != "connected" {
        return Err(invalid(format!(
            "{} is not connected in {}; connect it there, then ask again",
            connection.server,
            router::agent_label(&connection.harness)
        )));
    }
    let open_id = uuid::Uuid::new_v4().to_string();
    core.try_db()?.execute(
        "INSERT INTO personal_app_opens(open_id,session_id,app_id,version,content_hash,prefill,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![
            open_id,
            session,
            version.app_id,
            version.version,
            version.content_hash,
            input.prefill.as_ref().map(Value::to_string),
            Utc::now().to_rfc3339()
        ],
    )?;
    Ok(json!({
        "openId": open_id,
        "appId": version.app_id,
        "version": version.version,
        "contentHash": version.content_hash,
        "resourceUri": format!("ui://bridge/personal/{}@{}", version.app_id, version.content_hash),
        "note": "The person sees and uses the app in the card. Reads run live; every write waits for the person to approve the exact call.",
    }))
}

struct Opened {
    version: LockedVersion,
    prefill: Option<Value>,
}

fn load(db: &Db, session: &str, open_id: &str, content_hash: &str) -> Result<Opened, BridgeError> {
    if !super::draft::eligible(db, session)? {
        return Err(invalid("Personal apps need a top-level chat in Claude Code, Codex, OpenCode or Cursor"));
    }
    let (app_id, prefill): (String, Option<String>) = db
        .query_row(
            "SELECT app_id, prefill FROM personal_app_opens WHERE open_id=?1 AND session_id=?2 AND content_hash=?3",
            params![open_id, session, content_hash],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| invalid("This app is unavailable in this chat"))?;
    let version = personal::locked_with_hash(db, &app_id, content_hash)?
        .ok_or_else(|| invalid("This app version was deleted"))?;
    let prefill = prefill.map(|text| serde_json::from_str(&text)).transpose().map_err(|_| invalid("This app's prefill is unreadable"))?;
    Ok(Opened { version: verified(version)?, prefill })
}

pub fn get(db: &Db, p: &GetPersonalAppOpenParams) -> Result<PersonalAppOpen, BridgeError> {
    let Opened { version, prefill } = load(db, &p.session_id, &p.open_id, &p.content_hash)?;
    let manifest = version.manifest;
    Ok(PersonalAppOpen {
        open_id: p.open_id.clone(),
        app_id: version.app_id,
        version: version.version,
        content_hash: version.content_hash,
        name: manifest.name,
        intent: manifest.intent,
        harness: manifest.harness,
        server: manifest.server,
        reads: manifest.reads,
        writes: manifest.writes,
        view_html: manifest.view_html,
        prefill,
    })
}

fn digest(text: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(text.as_bytes()))
}

/// One call from an opened app's view.
pub fn call(core: &Arc<BridgeCore>, p: &CallPersonalAppToolParams) -> Result<PersonalAppCallResult, BridgeError> {
    let opened = load(&*core.try_db()?, &p.session_id, &p.open_id, &p.content_hash)?;
    let manifest = &opened.version.manifest;
    let connection = connection(manifest);
    let started = Instant::now();
    let ledger = |kind: &str, ok: bool| record(core, kind, &opened.version, &p.tool, ok, started);
    if manifest.reads.contains(&p.tool) {
        check_arguments(&p.arguments)?;
        let result = router::read(core, &p.session_id, &connection, &p.tool, &p.arguments);
        ledger("read", result.is_ok());
        return result.map(|result| PersonalAppCallResult::Result { result });
    }
    if !manifest.writes.contains(&p.tool) {
        ledger("refused", false);
        return Err(invalid("This tool is not part of the app"));
    }
    match &p.approval_token {
        None => {
            check_arguments(&p.arguments)?;
            let token = offer(core, p, now())?;
            Ok(PersonalAppCallResult::ApprovalRequired {
                token: token.0,
                harness: manifest.harness.clone(),
                server: manifest.server.clone(),
                tool: p.tool.clone(),
                arguments_json: token.1,
            })
        }
        Some(token) => {
            let arguments = claim(&*core.try_db()?, p, token, now())?;
            let result = router::write(core, &p.session_id, &connection, &p.tool, &arguments);
            ledger("write", result.is_ok());
            result.map(|result| PersonalAppCallResult::Result { result })
        }
    }
}

fn now() -> DateTime<Utc> {
    Utc::now()
}

fn check_arguments(arguments: &Value) -> Result<(), BridgeError> {
    if !arguments.is_object() || arguments.to_string().len() > MAX_ARGUMENT_BYTES {
        return Err(invalid("Tool arguments must be an object within 64 KiB"));
    }
    Ok(())
}

/// Hold one write behind a fresh token. The canonical JSON is computed once,
/// here; it is what the sheet shows and what approving sends. A newer write
/// from the same open replaces an older pending one.
fn offer(core: &Arc<BridgeCore>, p: &CallPersonalAppToolParams, now: DateTime<Utc>) -> Result<(String, String), BridgeError> {
    let canonical = crate::mcp_apps::canonical_json(&p.arguments);
    let token = uuid::Uuid::new_v4().to_string();
    let db = core.try_db()?;
    let tx = db.unchecked_transaction()?;
    tx.execute("DELETE FROM personal_app_write_approvals WHERE open_id=?1", [&p.open_id])?;
    tx.execute(
        "INSERT INTO personal_app_write_approvals(token,open_id,session_id,tool,arguments,digest,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![token, p.open_id, p.session_id, p.tool, canonical, digest(&canonical), now.to_rfc3339()],
    )?;
    tx.commit()?;
    Ok((token, canonical))
}

/// Use a token once: it must be this open's, this chat's and this tool's, be
/// fresh, and hold arguments that still hash to their digest. Returns the
/// arguments parsed from the stored canonical text.
fn claim(db: &Db, p: &CallPersonalAppToolParams, token: &str, now: DateTime<Utc>) -> Result<Value, BridgeError> {
    let tx = db.unchecked_transaction()?;
    let row: Option<(String, String, String, String)> = tx
        .query_row(
            "SELECT tool, arguments, digest, created_at FROM personal_app_write_approvals WHERE token=?1 AND open_id=?2 AND session_id=?3",
            params![token, p.open_id, p.session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    tx.execute("DELETE FROM personal_app_write_approvals WHERE token=?1", [token])?;
    tx.commit()?;
    let (tool, arguments, stored_digest, created) = row.ok_or_else(|| invalid("This approval was already used or no longer exists"))?;
    let created = DateTime::parse_from_rfc3339(&created).map_err(|_| invalid("This approval is unreadable"))?.with_timezone(&Utc);
    if now - created > Duration::seconds(APPROVAL_TTL_SECS) {
        return Err(invalid("This approval expired; try the action again"));
    }
    if tool != p.tool || digest(&arguments) != stored_digest {
        return Err(invalid("This approval does not match the call"));
    }
    serde_json::from_str(&arguments).map_err(|_| invalid("This approval is unreadable"))
}

fn record(core: &Arc<BridgeCore>, kind: &str, version: &LockedVersion, tool: &str, ok: bool, started: Instant) {
    let body = json!({
        "version": version.version,
        "tool": tool,
        "ok": ok,
        "elapsedMs": u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    });
    let recorded = core.try_db().and_then(|db| {
        crate::store::event(&db, "personal_app", &format!("personal_app.call.{kind}"), &version.app_id, &body.to_string())
    });
    if let Err(error) = recorded {
        crate::diagnostics::record(&format!("personal app ledger: a call was not recorded: {error}"));
    }
}

#[cfg(test)]
mod tests;

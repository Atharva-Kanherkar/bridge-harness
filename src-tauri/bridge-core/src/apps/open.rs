//! Opening a locked personal app, and every call its view makes.
//!
//! An open has an owner, stored with it:
//! - a **chat**: `apps_open` binds one chat to one exact locked version (its
//!   content hash) and the model's prefill. Reads follow the chat's access
//!   mode, and the chat's own process may carry calls;
//! - a **Surface**: the person opened the app from the Store or the launcher
//!   (`apps/open_personal_app_surface`, a protocol method only the Bridge UI
//!   calls; no MCP tool reaches it). Reads are allowed, calls never ride a
//!   chat's process, and there is at most one per app.
//!
//! A caller names the owner by passing a session id or none, and it must equal
//! the stored one, so no caller can pick the more permissive owner. Every open
//! carries the app's generation; a deleted and recreated app is a new
//! generation, and old opens never attach to it. The card reloads that binding, never the model's
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
    registry,
    router::{self, Caller, Connection},
};
use crate::{runtime::BridgeCore, BridgeError};
use bridge_protocol::messages::{CallPersonalAppToolParams, GetPersonalAppOpenParams, PersonalAppCallResult, PersonalAppOpen, PersonalAppSurfaceOpen};
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

/// Migration 84: opens get an owner and a generation, and every locked app a
/// generation. Pending approvals are dropped; they live ten minutes at most.
pub fn install_owners(tx: &Transaction<'_>) -> Result<(), BridgeError> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS personal_app_generations(
            app_id TEXT PRIMARY KEY, generation TEXT NOT NULL, created_at TEXT NOT NULL
        );
        INSERT OR IGNORE INTO personal_app_generations(app_id, generation, created_at)
            SELECT DISTINCT app_id, lower(hex(randomblob(16))), datetime('now') FROM personal_app_versions;
        DROP TABLE IF EXISTS personal_app_write_approvals;
        CREATE TABLE personal_app_opens_v2(
            open_id TEXT PRIMARY KEY,
            owner TEXT NOT NULL CHECK(owner IN ('chat','surface')),
            session_id TEXT,
            app_id TEXT NOT NULL, generation TEXT NOT NULL,
            version INTEGER NOT NULL, content_hash TEXT NOT NULL, prefill TEXT, created_at TEXT NOT NULL,
            CHECK((owner='chat' AND session_id IS NOT NULL) OR (owner='surface' AND session_id IS NULL))
        );
        INSERT INTO personal_app_opens_v2
            SELECT o.open_id, 'chat', o.session_id, o.app_id, g.generation, o.version, o.content_hash, o.prefill, o.created_at
            FROM personal_app_opens o JOIN personal_app_generations g ON g.app_id = o.app_id;
        DROP TABLE personal_app_opens;
        ALTER TABLE personal_app_opens_v2 RENAME TO personal_app_opens;
        CREATE INDEX personal_app_opens_session ON personal_app_opens(session_id);
        CREATE INDEX personal_app_opens_app ON personal_app_opens(app_id);
        CREATE TABLE personal_app_write_approvals(
            token TEXT PRIMARY KEY, open_id TEXT NOT NULL UNIQUE,
            tool TEXT NOT NULL, arguments TEXT NOT NULL, digest TEXT NOT NULL, created_at TEXT NOT NULL
        );",
    )?;
    Ok(())
}

/// The locked apps a chat's `apps_open` description names: id, name, intent.
/// Apps that are off are left out.
pub fn listing(db: &Db) -> Result<Vec<Value>, BridgeError> {
    Ok(personal::list(db)?
        .into_iter()
        .filter(|app| registry::is_enabled(db, &app.id).unwrap_or(false))
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
    let (version, generation) = openable(core, &input.app_id, Caller::Chat(session))?;
    let open_id = uuid::Uuid::new_v4().to_string();
    core.try_db()?.execute(
        "INSERT INTO personal_app_opens(open_id,owner,session_id,app_id,generation,version,content_hash,prefill,created_at)
         VALUES(?1,'chat',?2,?3,?4,?5,?6,?7,?8)",
        params![
            open_id,
            session,
            version.app_id,
            generation,
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

/// The latest locked, hash-verified version of an app that is on, its current
/// generation, and proof its server is connected for `caller` to open it.
fn openable(core: &Arc<BridgeCore>, app_id: &str, caller: Caller<'_>) -> Result<(LockedVersion, String), BridgeError> {
    let (version, generation) = {
        let db = core.try_db()?;
        let found = personal::latest_locked(&db, app_id)?;
        let version = found.ok_or_else(|| unknown_app(&db, app_id))?;
        if !registry::is_enabled(&db, app_id)? {
            return Err(invalid(TURNED_OFF));
        }
        let generation = personal::generation(&db, app_id)?.ok_or_else(|| invalid(DELETED))?;
        (version, generation)
    };
    let version = verified(version)?;
    let connection = connection(&version.manifest);
    let inventory = router::inventory(core, caller, &connection)?;
    if inventory["status"] != "connected" {
        return Err(invalid(format!(
            "{} is not connected in {}; connect it there, then try again",
            connection.server,
            router::agent_label(&connection.harness)
        )));
    }
    Ok((version, generation))
}

const TURNED_OFF: &str = "This app is turned off. Turn it on in the Store to use it";
const DELETED: &str = "This app was deleted";

/// `apps/open_personal_app_surface`: the person opens an app themselves. It
/// replaces the app's previous Surface open, and with it any write waiting
/// on that open.
pub fn open_surface(core: &Arc<BridgeCore>, app_id: &str) -> Result<PersonalAppSurfaceOpen, BridgeError> {
    personal::validate_id(app_id).map_err(invalid)?;
    let (version, generation) = openable(core, app_id, Caller::Surface)?;
    let open_id = uuid::Uuid::new_v4().to_string();
    let db = core.try_db()?;
    let tx = db.unchecked_transaction()?;
    // The app may have been turned off or deleted while its server was asked.
    if personal::generation(&tx, app_id)?.as_deref() != Some(generation.as_str()) {
        return Err(invalid(DELETED));
    }
    if !registry::is_enabled(&tx, app_id)? {
        return Err(invalid(TURNED_OFF));
    }
    remove_surfaces(&tx, Some(app_id))?;
    tx.execute(
        "INSERT INTO personal_app_opens(open_id,owner,session_id,app_id,generation,version,content_hash,prefill,created_at)
         VALUES(?1,'surface',NULL,?2,?3,?4,?5,NULL,?6)",
        params![open_id, version.app_id, generation, version.version, version.content_hash, Utc::now().to_rfc3339()],
    )?;
    tx.commit()?;
    Ok(PersonalAppSurfaceOpen { open_id, app_id: version.app_id, version: version.version, content_hash: version.content_hash })
}

/// `apps/close_personal_app_surface`: the pane closed. `true` when it was open.
pub fn close_surface(db: &Db, open_id: &str) -> Result<bool, BridgeError> {
    let tx = db.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM personal_app_write_approvals WHERE open_id IN (SELECT open_id FROM personal_app_opens WHERE open_id=?1 AND owner='surface')",
        [open_id],
    )?;
    let removed = tx.execute("DELETE FROM personal_app_opens WHERE open_id=?1 AND owner='surface'", [open_id])? > 0;
    tx.commit()?;
    Ok(removed)
}

/// Surface opens and their pending writes, for one app or (at daemon start)
/// all of them.
pub fn remove_surfaces(db: &Db, app_id: Option<&str>) -> Result<(), BridgeError> {
    db.execute(
        "DELETE FROM personal_app_write_approvals WHERE open_id IN
         (SELECT open_id FROM personal_app_opens WHERE owner='surface' AND (?1 IS NULL OR app_id=?1))",
        [app_id],
    )?;
    db.execute("DELETE FROM personal_app_opens WHERE owner='surface' AND (?1 IS NULL OR app_id=?1)", [app_id])?;
    Ok(())
}

/// The app was turned off: every write waiting on it is revoked and its
/// Surface open closed, in the caller's transaction. Chat opens stay, and
/// are refused until it is on again.
pub fn revoke_app(db: &Db, app_id: &str) -> Result<(), BridgeError> {
    db.execute(
        "DELETE FROM personal_app_write_approvals WHERE open_id IN (SELECT open_id FROM personal_app_opens WHERE app_id=?1)",
        [app_id],
    )?;
    remove_surfaces(db, Some(app_id))
}

fn caller(session: Option<&str>) -> Caller<'_> {
    session.map_or(Caller::Surface, Caller::Chat)
}

struct Opened {
    version: LockedVersion,
    generation: String,
    prefill: Option<Value>,
}

/// The app an open is for must still be this generation and on.
fn still_usable(db: &Db, app_id: &str, generation: &str) -> Result<(), BridgeError> {
    if personal::generation(db, app_id)?.as_deref() != Some(generation) {
        return Err(invalid(DELETED));
    }
    if !registry::is_enabled(db, app_id)? {
        return Err(invalid(TURNED_OFF));
    }
    Ok(())
}

fn load(db: &Db, owner: Caller<'_>, open_id: &str, content_hash: &str) -> Result<Opened, BridgeError> {
    let session = match owner {
        Caller::Chat(session) => {
            if !super::draft::eligible(db, session)? {
                return Err(invalid("Personal apps need a top-level chat in Claude Code, Codex, OpenCode or Cursor"));
            }
            Some(session)
        }
        Caller::Surface => None,
    };
    let (app_id, generation, prefill): (String, String, Option<String>) = db
        .query_row(
            "SELECT app_id, generation, prefill FROM personal_app_opens
             WHERE open_id=?1 AND content_hash=?2
               AND ((owner='chat' AND session_id=?3) OR (owner='surface' AND ?3 IS NULL))",
            params![open_id, content_hash, session],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?
        .ok_or_else(|| invalid("This app is unavailable here"))?;
    still_usable(db, &app_id, &generation)?;
    let version = personal::locked_with_hash(db, &app_id, content_hash)?
        .ok_or_else(|| invalid("This app version was deleted"))?;
    let prefill = prefill.map(|text| serde_json::from_str(&text)).transpose().map_err(|_| invalid("This app's prefill is unreadable"))?;
    Ok(Opened { version: verified(version)?, generation, prefill })
}

pub fn get(db: &Db, p: &GetPersonalAppOpenParams) -> Result<PersonalAppOpen, BridgeError> {
    let Opened { version, prefill, .. } = load(db, caller(p.session_id.as_deref()), &p.open_id, &p.content_hash)?;
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
    let owner = caller(p.session_id.as_deref());
    let opened = load(&*core.try_db()?, owner, &p.open_id, &p.content_hash)?;
    let manifest = &opened.version.manifest;
    let connection = connection(manifest);
    let started = Instant::now();
    let ledger = |kind: &str, outcome: Outcome| record(core, kind, &opened, &p.tool, outcome, started);
    if manifest.reads.contains(&p.tool) {
        check_arguments(&p.arguments)?;
        let result = router::read(core, owner, &connection, &p.tool, &p.arguments);
        // A read changes nothing, so a read that errored simply failed.
        ledger("read", Outcome::of(&result, false));
        return result.map(|result| PersonalAppCallResult::Result { result });
    }
    if !manifest.writes.contains(&p.tool) {
        ledger("refused", Outcome::Failed);
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
            if let Err(error) = router::preflight_write(core, &connection) {
                ledger("write", Outcome::Failed);
                return Err(error);
            }
            let result = router::write(core, owner, &connection, &p.tool, &arguments);
            let outcome = Outcome::of(&result, true);
            ledger("write", outcome);
            match result {
                Ok(result) => Ok(PersonalAppCallResult::Result { result }),
                Err(error) => Err(invalid(format!("Bridge could not confirm whether this was sent: {error}"))),
            }
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
        "INSERT INTO personal_app_write_approvals(token,open_id,tool,arguments,digest,created_at) VALUES(?1,?2,?3,?4,?5,?6)",
        params![token, p.open_id, p.tool, canonical, digest(&canonical), now.to_rfc3339()],
    )?;
    tx.commit()?;
    Ok((token, canonical))
}

/// Use a token once: it must be this open's, this owner's and this tool's, be
/// fresh, and hold arguments that still hash to their digest. The app must
/// still be this generation and on, checked in the same transaction that
/// uses the token, so turning it off or deleting it either comes first and
/// refuses this, or comes after the write was already let through. Returns
/// the arguments parsed from the stored canonical text.
fn claim(db: &Db, p: &CallPersonalAppToolParams, token: &str, now: DateTime<Utc>) -> Result<Value, BridgeError> {
    let tx = db.unchecked_transaction()?;
    let row: Option<(String, String, String, String, String, String)> = tx
        .query_row(
            "SELECT a.tool, a.arguments, a.digest, a.created_at, o.app_id, o.generation
             FROM personal_app_write_approvals a JOIN personal_app_opens o ON o.open_id = a.open_id
             WHERE a.token=?1 AND a.open_id=?2
               AND ((o.owner='chat' AND o.session_id=?3) OR (o.owner='surface' AND ?3 IS NULL))",
            params![token, p.open_id, p.session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )
        .optional()?;
    tx.execute("DELETE FROM personal_app_write_approvals WHERE token=?1", [token])?;
    let usable = match &row {
        Some((.., app_id, generation)) => still_usable(&tx, app_id, generation),
        None => Ok(()),
    };
    tx.commit()?;
    usable?;
    let (tool, arguments, stored_digest, created, ..) = row.ok_or_else(|| invalid("This approval was already used or no longer exists"))?;
    let created = DateTime::parse_from_rfc3339(&created).map_err(|_| invalid("This approval is unreadable"))?.with_timezone(&Utc);
    if now - created > Duration::seconds(APPROVAL_TTL_SECS) {
        return Err(invalid("This approval expired; try the action again"));
    }
    if tool != p.tool || digest(&arguments) != stored_digest {
        return Err(invalid("This approval does not match the call"));
    }
    serde_json::from_str(&arguments).map_err(|_| invalid("This approval is unreadable"))
}

/// What became of a call, as far as Bridge can know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The server answered without an MCP error.
    Confirmed,
    /// Refused before anything was sent, or the server answered `isError`.
    Failed,
    /// Sending began and then the transport failed: it may have run.
    Unknown,
}

impl Outcome {
    /// Adapters return a server's MCP error inside `Ok`, so `Ok` alone does
    /// not mean it worked. `dispatched` is whether an `Err` came after
    /// sending began.
    pub fn of(result: &Result<Value, BridgeError>, dispatched: bool) -> Self {
        match result {
            Ok(value) if value.get("isError") == Some(&Value::Bool(true)) => Self::Failed,
            Ok(_) => Self::Confirmed,
            Err(_) if dispatched => Self::Unknown,
            Err(_) => Self::Failed,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
        }
    }
}

fn record(core: &Arc<BridgeCore>, kind: &str, opened: &Opened, tool: &str, outcome: Outcome, started: Instant) {
    let version = &opened.version;
    let body = json!({
        "version": version.version,
        "generation": opened.generation,
        "tool": tool,
        "outcome": outcome.as_str(),
        "ok": outcome == Outcome::Confirmed,
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

//! Personal apps: views the person's own harness built for one MCP server.
//!
//! An app names one server on one harness, the tools its view may call
//! without asking (`reads`) and the tools it may call only after the person
//! approves the exact call (`writes`). Bridge never holds the server's
//! credential: the harness that owns the connection makes every call.
//!
//! The person locks each version, and a lock binds to what they previewed: a
//! draft's id and its content hash. A version that can do more than the last
//! one (a new read, a new write, another server) is reported and stored only
//! when the person allows the wider version. Narrowing is silent.

use std::collections::BTreeSet;

use bridge_protocol::messages::{LockPersonalAppResult, PersonalApp, PersonalAppSummary};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::BridgeError;

pub const MAX_NAME_CHARS: usize = 60;
pub const MAX_INTENT_CHARS: usize = 300;
pub const MAX_SERVER_CHARS: usize = 100;
pub const MAX_TOOLS: usize = 40;
pub const MAX_TOOL_CHARS: usize = 64;
pub const MAX_VIEW_BYTES: usize = 256 * 1024;
/// How long a draft can wait for its lock.
pub const DRAFT_TTL_HOURS: i64 = 24;
/// Harnesses an app may name: the ones Bridge can make a direct tool call on.
pub const HARNESSES: &[&str] = &["claude"];

pub fn install_store(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS personal_app_drafts(
            id TEXT PRIMARY KEY,
            app_id TEXT NOT NULL,
            manifest TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS personal_app_versions(
            app_id TEXT NOT NULL,
            version INTEGER NOT NULL,
            manifest TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            locked_at TEXT NOT NULL,
            PRIMARY KEY (app_id, version)
        );",
    )?;
    Ok(())
}

/// What a personal app is, apart from its id and version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub name: String,
    pub intent: String,
    pub harness: String,
    pub server: String,
    pub reads: Vec<String>,
    pub writes: Vec<String>,
    pub view_html: String,
}

impl Manifest {
    /// The same manifest with its tool lists sorted, the form that is hashed
    /// and stored.
    fn normalized(&self) -> Self {
        let mut normal = self.clone();
        normal.reads.sort();
        normal.writes.sort();
        normal
    }
}

/// A stored draft: what slice 3's preview shows, waiting for its lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub id: String,
    pub app_id: String,
    pub manifest: Manifest,
    pub content_hash: String,
}

fn tool_name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_TOOL_CHARS
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn is_builtin_id(id: &str) -> bool {
    super::builtins().iter().any(|app| app.manifest.id == id)
}

/// Check an app id: kebab-case like every app, and never a built-in's id.
pub fn validate_id(id: &str) -> Result<(), String> {
    if id.chars().count() > super::manifest::MAX_ID_CHARS || !super::manifest::is_kebab(id) {
        return Err(format!("app id \"{id}\" must be kebab-case and at most {} characters", super::manifest::MAX_ID_CHARS));
    }
    if is_builtin_id(id) {
        return Err(format!("\"{id}\" is a built-in app"));
    }
    Ok(())
}

/// Check a manifest. The message names the rule it breaks.
pub fn validate(manifest: &Manifest) -> Result<(), String> {
    let name = manifest.name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("an app name must be 1 to {MAX_NAME_CHARS} characters"));
    }
    let intent = manifest.intent.trim();
    if intent.is_empty() || intent.chars().count() > MAX_INTENT_CHARS {
        return Err(format!("an app intent must be 1 to {MAX_INTENT_CHARS} characters"));
    }
    if !HARNESSES.contains(&manifest.harness.as_str()) {
        return Err(format!("personal apps run on {} only, not \"{}\"", HARNESSES.join(", "), manifest.harness));
    }
    let server = manifest.server.trim();
    if server.is_empty() || server.chars().count() > MAX_SERVER_CHARS || server != manifest.server {
        return Err(format!("an app server must be a configured server name of 1 to {MAX_SERVER_CHARS} characters"));
    }
    let total = manifest.reads.len() + manifest.writes.len();
    if total == 0 || total > MAX_TOOLS {
        return Err(format!("an app uses 1 to {MAX_TOOLS} tools"));
    }
    let mut seen = BTreeSet::new();
    for tool in manifest.reads.iter().chain(&manifest.writes) {
        if !tool_name_ok(tool) {
            return Err(format!("\"{tool}\" is not a bare lowercase tool name"));
        }
        if !seen.insert(tool.as_str()) {
            return Err(format!("the tool {tool} is listed twice"));
        }
    }
    // A read skips approval, so it must be a read by the vocabulary every
    // other gate in Bridge uses. `slack_send_message` is never a read.
    if let Some(tool) = manifest.reads.iter().find(|tool| !crate::briefing_policy::is_read_tool_name(tool)) {
        return Err(format!("{tool} is not a read tool; list it as a write so it asks first"));
    }
    if manifest.view_html.trim().is_empty() || manifest.view_html.len() > MAX_VIEW_BYTES {
        return Err(format!("an app view must be 1 byte to {} KiB of HTML", MAX_VIEW_BYTES / 1024));
    }
    Ok(())
}

/// `sha256:` over the canonical JSON of the manifest with its tools sorted.
pub fn content_hash(manifest: &Manifest) -> String {
    let value = serde_json::to_value(manifest.normalized()).unwrap_or_default();
    let canonical = crate::mcp_apps::canonical_json(&value);
    format!("sha256:{:x}", Sha256::digest(canonical.as_bytes()))
}

/// A kebab-case id from a name, at most 40 characters, `app` when nothing is
/// left.
fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let mut out: String = out.chars().take(super::manifest::MAX_ID_CHARS - 3).collect();
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() { "app".into() } else { out }
}

/// An id for a new app: its name's slug, numbered past any personal or
/// built-in app that already has it.
fn fresh_id(db: &Connection, name: &str) -> Result<String, BridgeError> {
    let base = slug(name);
    for n in 1..1000 {
        let id = if n == 1 { base.clone() } else { format!("{base}-{n}") };
        let taken: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM personal_app_versions WHERE app_id=?1)",
            [&id],
            |row| row.get(0),
        )?;
        if !taken && !is_builtin_id(&id) {
            return Ok(id);
        }
    }
    Err(BridgeError::Invalid("too many apps share this name".into()))
}

fn prune_drafts(db: &Connection, now: DateTime<Utc>) -> Result<(), BridgeError> {
    let cutoff = (now - Duration::hours(DRAFT_TTL_HOURS)).to_rfc3339();
    db.execute("DELETE FROM personal_app_drafts WHERE created_at < ?1", [cutoff])?;
    Ok(())
}

/// Store a draft for the person to preview and lock. `app_id` names the app a
/// new version is for; `None` starts a new app.
pub fn create_draft(
    db: &Connection,
    app_id: Option<&str>,
    manifest: &Manifest,
    now: DateTime<Utc>,
) -> Result<Draft, BridgeError> {
    validate(manifest).map_err(BridgeError::Invalid)?;
    prune_drafts(db, now)?;
    let app_id = match app_id {
        Some(id) => {
            validate_id(id).map_err(BridgeError::Invalid)?;
            id.to_owned()
        }
        None => fresh_id(db, &manifest.name)?,
    };
    let manifest = manifest.normalized();
    let draft = Draft {
        id: Uuid::new_v4().to_string(),
        app_id,
        content_hash: content_hash(&manifest),
        manifest,
    };
    db.execute(
        "INSERT INTO personal_app_drafts(id,app_id,manifest,content_hash,created_at) VALUES(?1,?2,?3,?4,?5)",
        params![
            draft.id,
            draft.app_id,
            serde_json::to_string(&draft.manifest).map_err(|error| BridgeError::Invalid(error.to_string()))?,
            draft.content_hash,
            now.to_rfc3339()
        ],
    )?;
    Ok(draft)
}

struct Version {
    app_id: String,
    version: u32,
    manifest: Manifest,
    content_hash: String,
    locked_at: String,
}

impl Version {
    fn summary(&self) -> PersonalAppSummary {
        PersonalAppSummary {
            id: self.app_id.clone(),
            version: self.version,
            name: self.manifest.name.clone(),
            intent: self.manifest.intent.clone(),
            harness: self.manifest.harness.clone(),
            server: self.manifest.server.clone(),
            reads: self.manifest.reads.clone(),
            writes: self.manifest.writes.clone(),
            content_hash: self.content_hash.clone(),
            view_bytes: u32::try_from(self.manifest.view_html.len()).unwrap_or(u32::MAX),
            locked_at: self.locked_at.clone(),
        }
    }
}

fn parse_manifest(text: &str) -> Result<Manifest, BridgeError> {
    serde_json::from_str(text).map_err(|error| BridgeError::Invalid(format!("a stored personal app is unreadable: {error}")))
}

fn latest(db: &Connection, app_id: &str) -> Result<Option<Version>, BridgeError> {
    let row = db
        .query_row(
            "SELECT version, manifest, content_hash, locked_at FROM personal_app_versions
             WHERE app_id=?1 ORDER BY version DESC LIMIT 1",
            [app_id],
            |row| Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?)),
        )
        .optional()?;
    row.map(|(version, manifest, content_hash, locked_at)| {
        Ok(Version { app_id: app_id.to_owned(), version, manifest: parse_manifest(&manifest)?, content_hash, locked_at })
    })
    .transpose()
}

/// What a draft can do that the locked version could not.
fn widening(before: &Manifest, after: &Manifest) -> Option<LockPersonalAppResult> {
    let reads: BTreeSet<&String> = before.reads.iter().collect();
    let callable: BTreeSet<&String> = before.reads.iter().chain(&before.writes).collect();
    // A tool that was a write and is now a read stops asking: that widens.
    // A read that becomes a write starts asking: that narrows.
    let added_reads: Vec<String> = after.reads.iter().filter(|tool| !reads.contains(tool)).cloned().collect();
    let added_writes: Vec<String> = after.writes.iter().filter(|tool| !callable.contains(tool)).cloned().collect();
    let target_changed = before.harness != after.harness || before.server != after.server;
    (!added_reads.is_empty() || !added_writes.is_empty() || target_changed)
        .then_some(LockPersonalAppResult::Widens { added_reads, added_writes, target_changed })
}

/// Lock a draft the person previewed. See the module docs for the rules.
pub fn lock(
    db: &Connection,
    draft_id: &str,
    expected_hash: &str,
    allow_wider: bool,
    now: DateTime<Utc>,
) -> Result<LockPersonalAppResult, BridgeError> {
    prune_drafts(db, now)?;
    let draft = db
        .query_row(
            "SELECT app_id, manifest, content_hash FROM personal_app_drafts WHERE id=?1",
            [draft_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
        )
        .optional()?
        .ok_or_else(|| BridgeError::Invalid("this app draft no longer exists; ask for it again".into()))?;
    let (app_id, manifest, stored_hash) = draft;
    let manifest = parse_manifest(&manifest)?;
    // The draft must still be what was previewed, and still hash to itself.
    if expected_hash != stored_hash || content_hash(&manifest) != stored_hash {
        return Err(BridgeError::Invalid("this app changed after you previewed it; preview it again".into()));
    }
    validate(&manifest).map_err(BridgeError::Invalid)?;
    let previous = latest(db, &app_id)?;
    if let Some(previous) = &previous {
        if previous.content_hash == stored_hash {
            db.execute("DELETE FROM personal_app_drafts WHERE id=?1", [draft_id])?;
            return Ok(LockPersonalAppResult::Unchanged { app: previous.summary() });
        }
        if let Some(widens) = widening(&previous.manifest, &manifest) {
            if !allow_wider {
                return Ok(widens);
            }
        }
    }
    let version = Version {
        app_id,
        version: previous.as_ref().map_or(1, |previous| previous.version + 1),
        manifest,
        content_hash: stored_hash,
        locked_at: now.to_rfc3339(),
    };
    let transaction = db.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO personal_app_versions(app_id,version,manifest,content_hash,locked_at) VALUES(?1,?2,?3,?4,?5)",
        params![
            version.app_id,
            version.version,
            serde_json::to_string(&version.manifest).map_err(|error| BridgeError::Invalid(error.to_string()))?,
            version.content_hash,
            version.locked_at
        ],
    )?;
    transaction.execute("DELETE FROM personal_app_drafts WHERE id=?1", [draft_id])?;
    transaction.commit()?;
    Ok(LockPersonalAppResult::Locked { app: version.summary() })
}

/// The latest version of every app, most recently locked first.
pub fn list(db: &Connection) -> Result<Vec<PersonalAppSummary>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT v.app_id, v.version, v.manifest, v.content_hash, v.locked_at
         FROM personal_app_versions v
         WHERE v.version = (SELECT MAX(version) FROM personal_app_versions WHERE app_id = v.app_id)
         ORDER BY v.locked_at DESC, v.app_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?))
    })?;
    let mut apps = Vec::new();
    for row in rows {
        let (app_id, version, manifest, content_hash, locked_at) = row?;
        apps.push(Version { app_id, version, manifest: parse_manifest(&manifest)?, content_hash, locked_at }.summary());
    }
    Ok(apps)
}

/// The latest version of one app, with its view.
pub fn get(db: &Connection, app_id: &str) -> Result<PersonalApp, BridgeError> {
    let version = latest(db, app_id)?.ok_or_else(|| BridgeError::Invalid(format!("no personal app \"{app_id}\"")))?;
    Ok(PersonalApp { app: version.summary(), view_html: version.manifest.view_html })
}

/// Remove every version and draft of an app. `true` when anything was removed.
pub fn delete(db: &Connection, app_id: &str) -> Result<bool, BridgeError> {
    let transaction = db.unchecked_transaction()?;
    let versions = transaction.execute("DELETE FROM personal_app_versions WHERE app_id=?1", [app_id])?;
    let drafts = transaction.execute("DELETE FROM personal_app_drafts WHERE app_id=?1", [app_id])?;
    transaction.commit()?;
    Ok(versions + drafts > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let mut db = Connection::open_in_memory().unwrap();
        let transaction = db.transaction().unwrap();
        install_store(&transaction).unwrap();
        transaction.commit().unwrap();
        db
    }

    fn now() -> DateTime<Utc> {
        "2026-11-16T10:00:00Z".parse().unwrap()
    }

    fn manifest() -> Manifest {
        Manifest {
            name: "Slack DM".into(),
            intent: "Message a teammate on Slack".into(),
            harness: "claude".into(),
            server: "claude.ai Slack".into(),
            reads: vec!["slack_search_users".into(), "slack_read_channel".into()],
            writes: vec!["slack_send_message".into()],
            view_html: "<form><button>Send</button></form>".into(),
        }
    }

    fn locked(db: &Connection, app_id: Option<&str>, manifest: &Manifest, allow_wider: bool) -> LockPersonalAppResult {
        let draft = create_draft(db, app_id, manifest, now()).unwrap();
        lock(db, &draft.id, &draft.content_hash, allow_wider, now()).unwrap()
    }

    #[test]
    fn a_valid_manifest_passes_and_each_rule_refuses() {
        assert_eq!(validate(&manifest()), Ok(()));
        let cases: Vec<(&str, Box<dyn Fn(&mut Manifest)>)> = vec![
            ("name", Box::new(|m| m.name = " ".into())),
            ("name", Box::new(|m| m.name = "n".repeat(61))),
            ("intent", Box::new(|m| m.intent = String::new())),
            ("intent", Box::new(|m| m.intent = "i".repeat(301))),
            ("claude", Box::new(|m| m.harness = "codex".into())),
            ("server", Box::new(|m| m.server = String::new())),
            ("server", Box::new(|m| m.server = " claude.ai Slack".into())),
            ("tools", Box::new(|m| { m.reads.clear(); m.writes.clear(); })),
            ("tools", Box::new(|m| m.writes = (0..40).map(|n| format!("tool_{n}")).collect())),
            ("bare", Box::new(|m| m.writes = vec!["mcp__claude_ai_Slack__slack_send_message".into()])),
            ("bare", Box::new(|m| m.writes = vec!["Slack_Send".into()])),
            ("twice", Box::new(|m| m.writes = vec!["slack_send_message".into(), "slack_send_message".into()])),
            ("twice", Box::new(|m| m.writes = vec!["slack_read_channel".into()])),
            ("not a read", Box::new(|m| m.reads.push("slack_send_message_draft".into()))),
            ("not a read", Box::new(|m| m.reads.push("slack_channels".into()))),
            ("view", Box::new(|m| m.view_html = "  ".into())),
            ("view", Box::new(|m| m.view_html = "x".repeat(MAX_VIEW_BYTES + 1))),
        ];
        for (expected, change) in cases {
            let mut bad = manifest();
            change(&mut bad);
            let error = validate(&bad).unwrap_err();
            assert!(error.contains(expected), "{expected:?} in {error:?}");
        }
    }

    #[test]
    fn ids_are_kebab_case_and_never_a_built_in() {
        assert!(validate_id("slack-dm").is_ok());
        for id in ["", "Slack", "slack_dm", "-a", "a-", "a--b", &"a".repeat(41), "notes", "outgoing"] {
            assert!(validate_id(id).is_err(), "{id:?}");
        }
        assert_eq!(slug("Slack DM — Yamada!"), "slack-dm-yamada");
        assert_eq!(slug("!!!"), "app");
        assert!(slug(&"word ".repeat(30)).chars().count() <= 37);
    }

    #[test]
    fn the_hash_ignores_tool_order_and_follows_every_field() {
        let base = content_hash(&manifest());
        assert!(base.starts_with("sha256:") && base.len() == 7 + 64);
        let mut reordered = manifest();
        reordered.reads.reverse();
        assert_eq!(content_hash(&reordered), base);
        let changes: Vec<Box<dyn Fn(&mut Manifest)>> = vec![
            Box::new(|m| m.name.push('!')),
            Box::new(|m| m.intent.push('!')),
            Box::new(|m| m.harness = "other".into()),
            Box::new(|m| m.server.push('!')),
            Box::new(|m| m.reads.pop().map(|_| ()).unwrap_or(())),
            Box::new(|m| m.writes.push("slack_add_reaction".into())),
            Box::new(|m| m.view_html.push(' ')),
        ];
        for change in changes {
            let mut changed = manifest();
            change(&mut changed);
            assert_ne!(content_hash(&changed), base);
        }
    }

    #[test]
    fn a_new_app_locks_as_version_one_with_an_id_from_its_name() {
        let db = db();
        let LockPersonalAppResult::Locked { app } = locked(&db, None, &manifest(), false) else { panic!("locked") };
        assert_eq!((app.id.as_str(), app.version), ("slack-dm", 1));
        assert_eq!(app.reads, vec!["slack_read_channel", "slack_search_users"], "stored sorted");
        assert_eq!(app.content_hash, content_hash(&manifest()));
        // A second app with the same name gets its own id, never an edit.
        let LockPersonalAppResult::Locked { app: second } = locked(&db, None, &manifest(), false) else { panic!() };
        assert_eq!((second.id.as_str(), second.version), ("slack-dm-2", 1));
        let drafts: i64 = db.query_row("SELECT COUNT(*) FROM personal_app_drafts", [], |row| row.get(0)).unwrap();
        assert_eq!(drafts, 0, "a lock consumes its draft");
    }

    #[test]
    fn the_same_content_is_unchanged_and_a_narrower_version_locks_silently() {
        let db = db();
        locked(&db, None, &manifest(), false);
        let LockPersonalAppResult::Unchanged { app } = locked(&db, Some("slack-dm"), &manifest(), false) else { panic!("unchanged") };
        assert_eq!(app.version, 1);
        let mut narrower = manifest();
        narrower.reads.retain(|tool| tool != "slack_read_channel");
        narrower.view_html = "<p>smaller</p>".into();
        let LockPersonalAppResult::Locked { app } = locked(&db, Some("slack-dm"), &narrower, false) else { panic!("locked") };
        assert_eq!(app.version, 2);
        // A read moved to a write starts asking: also narrower.
        let mut asks = narrower.clone();
        asks.reads.clear();
        asks.writes.push("slack_search_users".into());
        assert!(matches!(locked(&db, Some("slack-dm"), &asks, false), LockPersonalAppResult::Locked { app } if app.version == 3));
    }

    #[test]
    fn a_wider_version_is_reported_and_stored_only_when_allowed() {
        let db = db();
        locked(&db, None, &manifest(), false);
        let mut wider = manifest();
        wider.reads.push("slack_list_channels".into());
        wider.writes.push("slack_add_reaction".into());
        let draft = create_draft(&db, Some("slack-dm"), &wider, now()).unwrap();
        assert_eq!(
            lock(&db, &draft.id, &draft.content_hash, false, now()).unwrap(),
            LockPersonalAppResult::Widens {
                added_reads: vec!["slack_list_channels".into()],
                added_writes: vec!["slack_add_reaction".into()],
                target_changed: false,
            }
        );
        assert_eq!(get(&db, "slack-dm").unwrap().app.version, 1, "nothing stored");
        // The draft survives the report, so the person can allow it.
        let LockPersonalAppResult::Locked { app } = lock(&db, &draft.id, &draft.content_hash, true, now()).unwrap() else { panic!() };
        assert_eq!(app.version, 2);

    }

    #[test]
    fn a_write_that_becomes_a_read_stops_asking_and_counts_as_wider() {
        let db = db();
        let mut base = manifest();
        base.writes.push("slack_read_thread".into());
        locked(&db, None, &base, false);
        let mut moved = base.clone();
        moved.writes.retain(|tool| tool != "slack_read_thread");
        moved.reads.push("slack_read_thread".into());
        let draft = create_draft(&db, Some("slack-dm"), &moved, now()).unwrap();
        assert_eq!(
            lock(&db, &draft.id, &draft.content_hash, false, now()).unwrap(),
            LockPersonalAppResult::Widens { added_reads: vec!["slack_read_thread".into()], added_writes: vec![], target_changed: false }
        );
    }

    #[test]
    fn another_server_or_harness_counts_as_wider() {
        let db = db();
        locked(&db, None, &manifest(), false);
        let mut other = manifest();
        other.server = "claude.ai Slack Work".into();
        let draft = create_draft(&db, Some("slack-dm"), &other, now()).unwrap();
        assert_eq!(
            lock(&db, &draft.id, &draft.content_hash, false, now()).unwrap(),
            LockPersonalAppResult::Widens { added_reads: vec![], added_writes: vec![], target_changed: true }
        );
        let mut before = manifest();
        before.harness = "codex".into();
        assert!(widening(&before, &manifest()).is_some());
    }

    #[test]
    fn a_lock_binds_to_the_previewed_draft() {
        let db = db();
        let draft = create_draft(&db, None, &manifest(), now()).unwrap();
        assert!(lock(&db, &draft.id, "sha256:other", false, now()).unwrap_err().to_string().contains("previewed"));
        // Content edited under the draft no longer hashes to its own hash.
        let mut edited = draft.manifest.clone();
        edited.writes.push("slack_delete_message".into());
        db.execute("UPDATE personal_app_drafts SET manifest=?1 WHERE id=?2", params![serde_json::to_string(&edited).unwrap(), draft.id]).unwrap();
        assert!(lock(&db, &draft.id, &draft.content_hash, false, now()).unwrap_err().to_string().contains("previewed"));
        assert!(lock(&db, "missing", &draft.content_hash, false, now()).unwrap_err().to_string().contains("no longer exists"));
        assert!(list(&db).unwrap().is_empty());
    }

    #[test]
    fn drafts_expire_after_a_day() {
        let db = db();
        let draft = create_draft(&db, None, &manifest(), now()).unwrap();
        let later = now() + Duration::hours(DRAFT_TTL_HOURS) + Duration::seconds(1);
        assert!(lock(&db, &draft.id, &draft.content_hash, false, later).is_err());
        let fresh = create_draft(&db, None, &manifest(), now()).unwrap();
        assert!(lock(&db, &fresh.id, &fresh.content_hash, false, now() + Duration::hours(23)).is_ok());
    }

    #[test]
    fn invalid_drafts_and_ids_are_refused_before_storage() {
        let db = db();
        let mut bad = manifest();
        bad.reads.push("slack_send_message_now".into());
        assert!(create_draft(&db, None, &bad, now()).is_err());
        assert!(create_draft(&db, Some("notes"), &manifest(), now()).is_err());
        let drafts: i64 = db.query_row("SELECT COUNT(*) FROM personal_app_drafts", [], |row| row.get(0)).unwrap();
        assert_eq!(drafts, 0);
    }

    #[test]
    fn list_get_and_delete() {
        let db = db();
        locked(&db, None, &manifest(), false);
        let mut later_app = manifest();
        later_app.name = "Linear triage".into();
        later_app.server = "claude.ai Linear".into();
        later_app.reads = vec!["linear_search_issues".into()];
        later_app.writes = vec![];
        let draft = create_draft(&db, None, &later_app, now()).unwrap();
        lock(&db, &draft.id, &draft.content_hash, false, now() + Duration::minutes(1)).unwrap();
        let mut narrower = manifest();
        narrower.reads.pop();
        locked(&db, Some("slack-dm"), &narrower, false);

        let apps = list(&db).unwrap();
        assert_eq!(apps.iter().map(|app| (app.id.as_str(), app.version)).collect::<Vec<_>>(), vec![("linear-triage", 1), ("slack-dm", 2)]);
        let full = get(&db, "slack-dm").unwrap();
        assert_eq!(full.view_html, manifest().view_html);
        assert_eq!(full.app.view_bytes as usize, manifest().view_html.len());
        assert!(get(&db, "nope").is_err());

        create_draft(&db, Some("slack-dm"), &manifest(), now()).unwrap();
        assert!(delete(&db, "slack-dm").unwrap());
        assert!(!delete(&db, "slack-dm").unwrap(), "idempotent");
        assert!(get(&db, "slack-dm").is_err());
        let left: i64 = db.query_row("SELECT COUNT(*) FROM personal_app_drafts WHERE app_id='slack-dm'", [], |row| row.get(0)).unwrap();
        assert_eq!(left, 0);
        assert_eq!(list(&db).unwrap().len(), 1);
    }
}

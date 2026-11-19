//! Keep talking to an imported chat (#1061).
//!
//! An imported chat is history and never changes. Continuing it makes a new
//! ordinary chat that branches from the imported head (the session fork, with
//! the history copied under new ids). When Claude Code still has the session
//! and Bridge signs in where it lives, the new chat carries that Claude Code
//! session id, so its first start resumes it natively and the next reply
//! continues the same session. Otherwise the new chat starts fresh, seeded
//! with the copied history through the checkpoint path, and says why.

use crate::provider_accounts::{self, HostHomes};
use crate::runtime::BridgeCore;
use crate::BridgeError;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const NATIVE: &str = "native";
pub const HISTORY: &str = "history";

/// How an imported chat would continue, decided before the first send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// `native` or `history`.
    pub mode: &'static str,
    /// Why it cannot continue natively, for the person.
    pub reason: Option<String>,
    /// The Claude Code session the new chat resumes, and the account it
    /// signs in with (`None`: the user's own login).
    pub native: Option<(String, Option<String>)>,
}

impl Plan {
    fn history(reason: &str) -> Self {
        Self { mode: HISTORY, reason: Some(reason.into()), native: None }
    }
}

/// One place Bridge can sign in to Claude Code: an account (`None` is the
/// user's own login) and its config folder.
pub type ClaudeHome = (Option<String>, PathBuf);

/// The Claude config folders Bridge signs in with: the user's own, then each
/// Claude account in Settings.
pub fn claude_homes(db: &Connection) -> Result<Vec<ClaudeHome>, BridgeError> {
    let host = HostHomes::from_env();
    let mut homes = vec![(None, provider_accounts::claude_home(None, &host))];
    let advanced = crate::agent_config::state(db)?
        .harnesses
        .into_iter()
        .find(|item| item.id == "claude")
        .map(|item| item.advanced);
    if let Some(advanced) = advanced {
        for account in provider_accounts::accounts("claude", &advanced)? {
            let home = provider_accounts::claude_home(Some(&account), &host);
            homes.push((Some(account.id), home));
        }
    }
    Ok(homes)
}

/// Claude Code files a session under its project folder, named after the
/// working directory with every character that is not a letter or a digit
/// written as `-`.
fn project_folder(cwd: &str) -> String {
    cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// Where Claude Code keeps `session` for `cwd` under a config folder.
fn transcript_path(home: &Path, cwd: &str, session: &str) -> PathBuf {
    home.join("projects").join(project_folder(cwd)).join(format!("{session}.jsonl"))
}

/// Where Bridge signs in to each tool it can resume natively.
#[derive(Debug, Clone, Default)]
pub struct SignInHomes {
    pub claude: Vec<ClaudeHome>,
    pub codex: Vec<ClaudeHome>,
}

impl SignInHomes {
    pub fn claude_only(homes: &[ClaudeHome]) -> Self {
        Self { claude: homes.to_vec(), codex: Vec::new() }
    }
}

/// A tool whose sessions Bridge resumes natively: what to call it, and
/// whether a home still holds a session for a folder.
struct Resumable {
    label: &'static str,
    still_has: fn(home: &Path, cwd: &str, session: &str) -> bool,
}

const CLAUDE: Resumable = Resumable {
    label: "Claude Code",
    still_has: |home, cwd, session| transcript_path(home, cwd, session).is_file(),
};

const CODEX: Resumable = Resumable {
    label: "Codex",
    // A Codex thread's rollout is filed by date, not by folder.
    still_has: |home, _cwd, session| crate::codex_import::find_rollout(home, session).is_some(),
};

pub fn plan(core: &BridgeCore, imported_id: &str) -> Result<Plan, BridgeError> {
    let db = core.db.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let homes = SignInHomes {
        claude: claude_homes(&db)?,
        codex: crate::codex_import::machine_codex_homes().into_iter().map(|home| (None, home)).collect(),
    };
    plan_in(&db, imported_id, &homes)
}

/// Decide how `imported_id` continues, against the given sign-in homes.
pub fn plan_in(db: &Connection, imported_id: &str, homes: &SignInHomes) -> Result<Plan, BridgeError> {
    let (kind, harness, cwd, parent): (String, String, Option<String>, Option<String>) = db
        .query_row(
            "SELECT kind, harness, cwd, parent_session_id FROM sessions WHERE id=?1",
            params![imported_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?
        .ok_or_else(|| BridgeError::Invalid("That chat does not exist".into()))?;
    if kind != "imported" {
        return Err(BridgeError::Invalid("Only an imported chat continues this way".into()));
    }
    let (tool, homes) = match harness.as_str() {
        "claude" => (CLAUDE, &homes.claude),
        "codex" => (CODEX, &homes.codex),
        _ => return Ok(Plan::history("Bridge continues only Claude Code and Codex sessions natively so far.")),
    };
    let label = tool.label;
    if parent.is_some() {
        return Ok(Plan::history("A subagent's run cannot be resumed on its own."));
    }
    let native: Option<String> = db
        .query_row(
            "SELECT native_key FROM import_native_sessions WHERE session_id=?1 AND native_key LIKE ?2 LIMIT 1",
            params![imported_id, format!("{harness}:%")],
            |row| row.get(0),
        )
        .optional()?;
    let prefix = format!("{harness}:");
    let Some(session) = native.and_then(|key| key.strip_prefix(&prefix).map(str::to_owned)) else {
        return Ok(Plan::history(&format!("Bridge did not record which {label} session this was.")));
    };
    let held: Option<String> = db
        .query_row(
            "SELECT id FROM sessions WHERE provider_session_id=?1 AND id<>?2 LIMIT 1",
            params![session, imported_id],
            |row| row.get(0),
        )
        .optional()?;
    if held.is_some() {
        return Ok(Plan::history(&format!("Another Bridge chat already continues this {label} session.")));
    }
    let Some(cwd) = cwd.filter(|cwd| Path::new(cwd).is_dir()) else {
        return Ok(Plan::history("This chat's folder no longer exists."));
    };
    if let Some((account, _)) = homes
        .iter()
        .find(|(_, home)| (tool.still_has)(home, &cwd, &session))
    {
        return Ok(Plan { mode: NATIVE, reason: None, native: Some((session, account.clone())) });
    }
    // Still on disk where it was imported from, but in a folder Bridge does
    // not sign in with: resuming there would start under another login.
    let source: Option<String> = db
        .query_row(
            "SELECT canonical_source_ref FROM imported_record_provenance WHERE bridge_id=?1 AND destination_kind='session'",
            params![imported_id],
            |row| row.get(0),
        )
        .optional()?;
    if source.is_some_and(|path| Path::new(&path).is_file()) {
        return Ok(Plan::history(&format!("The session is saved under a {label} folder Bridge does not sign in with.")));
    }
    Ok(Plan::history(&format!("{label} no longer has this session.")))
}

/// Continue `imported_id` in a new chat; returns its id and how it continues.
pub fn continue_chat(core: &Arc<BridgeCore>, imported_id: &str) -> Result<(String, Plan), BridgeError> {
    let plan = plan(core, imported_id)?;
    let id = {
        let db = core.db.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        continue_in(&db, imported_id, &plan)?
    };
    core.events.publish(crate::events::CoreEvent::StateChanged);
    Ok((id, plan))
}

pub(crate) fn continue_in(db: &Connection, imported_id: &str, plan: &Plan) -> Result<String, BridgeError> {
    let (head, title): (Option<String>, Option<String>) = db
        .query_row(
            "SELECT (SELECT active_entry_id FROM session_heads WHERE session_id=s.id), s.title FROM sessions s WHERE s.id=?1",
            params![imported_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| BridgeError::Invalid("That chat does not exist".into()))?;
    let head = head.ok_or_else(|| BridgeError::Invalid("This imported chat has no history to continue".into()))?;
    let id = crate::sessions::fork_session_records(db, imported_id, &head, title.as_deref(), None, None, "shared", None)?;
    if let Some((session, account)) = &plan.native {
        let transaction = db.unchecked_transaction()?;
        transaction.execute(
            "UPDATE sessions SET provider_session_id=?2, account=?3, continuation_fidelity='native' WHERE id=?1",
            params![id, session, account],
        )?;
        transaction.execute(
            "UPDATE session_heads SET native_provider_session_id=?2, restoration_mode='native', resume_eligibility='native' WHERE session_id=?1",
            params![id, session],
        )?;
        transaction.commit()?;
    }
    Ok(id)
}

#[cfg(test)]
#[path = "import_continue/tests.rs"]
mod tests;

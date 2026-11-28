//! Suggestions from the tool requests a person keeps answering the same way
//! (#1207). Suggest only: nothing here grants or refuses anything. An allow
//! the person gave again and again is pointed out with where to change chat
//! access; a decline is pointed out with an offer to write it down as a code
//! rule, which only ever asks agents to do less.
//!
//! A request is its tool and a short pattern: a shell command's first two
//! words (`cargo test`), or the top folder of the file it touches. A pattern
//! qualifies with at least [`MIN_SAME`] answers the same way across at least
//! [`MIN_SESSIONS`] chats and no opposite answer in [`QUIET_DAYS`] days.
//! Allows are never suggested for network, destructive or credential
//! commands.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, Transaction};
use serde::Serialize;

use super::signals::{self, Signal};
use crate::BridgeError;

pub const MIN_SAME: usize = 5;
pub const MIN_SESSIONS: usize = 2;
pub const QUIET_DAYS: i64 = 30;
/// Suggestions shown at once.
pub const MAX_SUGGESTIONS: usize = 3;

/// Schema 112: suggestions the person dismissed.
pub(crate) fn install(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS taste_dismissed_suggestions(
            key TEXT PRIMARY KEY,
            dismissed_at TEXT NOT NULL
        );",
    )?;
    Ok(())
}

/// Tools whose request body is a shell command.
const SHELL_TOOLS: &[&str] = &["bash", "shell", "exec_command", "run_shell_command", "command", "terminal"];

/// First words of commands an allow is never suggested for.
const RISKY: &[&str] = &[
    "curl", "wget", "ssh", "scp", "sftp", "rsync", "nc", "ncat", "telnet", "ftp", "http", "gh", "rm", "rmdir", "dd", "mkfs",
    "chmod", "chown", "sudo", "su", "kill", "killall", "pkill", "shutdown", "reboot", "truncate", "env", "printenv", "export",
    "security", "gpg", "aws", "gcloud", "az", "op", "kubectl", "docker", "eval", "sh", "bash", "zsh",
];

/// Two-word commands an allow is never suggested for.
const RISKY_PAIRS: &[&str] = &[
    "git push", "git reset", "git clean", "git checkout", "git rebase", "git filter-branch", "npm publish", "cargo publish",
    "pnpm publish", "yarn publish", "bun publish", "pip upload", "twine upload",
];

/// A request's tool and pattern.
pub fn pattern_of(signal: &Signal) -> Option<(String, String)> {
    let tool = signal.title.clone()?.trim().to_string();
    if tool.is_empty() {
        return None;
    }
    if SHELL_TOOLS.contains(&tool.to_lowercase().as_str()) {
        let command = signal.body.as_deref()?;
        let words: Vec<&str> = command
            .split_whitespace()
            .skip_while(|word| word.contains('=') && !word.starts_with('-'))
            .take(2)
            .collect();
        if words.is_empty() {
            return None;
        }
        let words: Vec<&str> = if words.len() == 2 && words[1].starts_with('-') { words[..1].to_vec() } else { words };
        return Some((tool, words.join(" ")));
    }
    let pattern = match signal.path.as_deref() {
        Some(path) => {
            let path = path.trim_start_matches("./");
            match path.split_once('/') {
                Some((top, _)) => format!("{top}/**"),
                None => path.to_string(),
            }
        }
        None => String::new(),
    };
    Some((tool, pattern))
}

/// Whether allowing `pattern` could reach the network, destroy something or
/// read credentials.
pub fn risky(pattern: &str) -> bool {
    let lower = pattern.to_lowercase();
    let first = lower.split_whitespace().next().unwrap_or_default();
    RISKY.contains(&first)
        || RISKY_PAIRS.iter().any(|pair| lower.starts_with(pair))
        || [".env", "secret", "token", "credential", "password", "id_rsa", ".ssh", "keychain"].iter().any(|word| lower.contains(word))
}

/// One suggestion.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    /// Stable for the same pattern, so a dismissal sticks.
    pub key: String,
    pub scope_key: String,
    pub tool: String,
    pub pattern: String,
    /// `allowed` or `declined`.
    pub verdict: String,
    pub count: usize,
    pub sessions: usize,
    pub last_at: String,
}

/// The suggestions worth showing now, most answered first.
pub fn suggestions(db: &Connection, now: DateTime<Utc>) -> Result<Vec<Suggestion>, BridgeError> {
    let dismissed: BTreeSet<String> = {
        let mut statement = db.prepare("SELECT key FROM taste_dismissed_suggestions")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        rows.collect::<Result<_, _>>()?
    };
    let permission: Vec<Signal> =
        signals::all(db)?.into_iter().filter(|signal| signal.source == signals::SOURCE_PERMISSION && signal.superseded_by.is_none()).collect();
    type Key = (String, String, String);
    let mut groups: BTreeMap<Key, Vec<&Signal>> = BTreeMap::new();
    for signal in &permission {
        if let Some((tool, pattern)) = pattern_of(signal) {
            groups.entry((signal.scope_key.clone(), tool, pattern)).or_default().push(signal);
        }
    }
    let quiet_since = signals::instant(&(now - Duration::days(QUIET_DAYS)).to_rfc3339());
    let mut found = Vec::new();
    for ((scope, tool, pattern), answers) in groups {
        for verdict in ["allowed", "declined"] {
            let same: Vec<&&Signal> = answers.iter().filter(|signal| signal.verdict == verdict).collect();
            let sessions: BTreeSet<&str> = same.iter().filter_map(|signal| signal.session_id.as_deref()).collect();
            let opposite_lately = answers.iter().any(|signal| signal.verdict != verdict && signal.at >= quiet_since);
            if same.len() < MIN_SAME || sessions.len() < MIN_SESSIONS || opposite_lately {
                continue;
            }
            if verdict == "allowed" && risky(&pattern) {
                continue;
            }
            let key = format!("{verdict}|{scope}|{tool}|{pattern}");
            if dismissed.contains(&key) {
                continue;
            }
            found.push(Suggestion {
                key,
                scope_key: scope.clone(),
                tool: tool.clone(),
                pattern: pattern.clone(),
                verdict: verdict.to_string(),
                count: same.len(),
                sessions: sessions.len(),
                last_at: same.iter().map(|signal| signal.at.clone()).max().unwrap_or_default(),
            });
        }
    }
    found.sort_by(|a, b| b.count.cmp(&a.count).then(b.last_at.cmp(&a.last_at)).then(a.key.cmp(&b.key)));
    found.truncate(MAX_SUGGESTIONS);
    Ok(found)
}

/// Stop showing a suggestion.
pub fn dismiss(db: &Connection, key: &str, now: DateTime<Utc>) -> Result<(), BridgeError> {
    if key.trim().is_empty() {
        return Err(BridgeError::Invalid("A suggestion needs its key to be dismissed.".into()));
    }
    db.execute(
        "INSERT OR IGNORE INTO taste_dismissed_suggestions(key, dismissed_at) VALUES(?1,?2)",
        params![key.trim(), signals::instant(&now.to_rfc3339())],
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "permissions_tests.rs"]
mod tests;

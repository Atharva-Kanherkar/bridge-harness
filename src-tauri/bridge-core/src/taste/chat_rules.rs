//! Coding agents get the person's code taste (#1204).
//!
//! A chat's memory packet carries the `code_prefer` rules the person approved
//! for its repository, and those for every repository: at most
//! [`MAX_TASTE_RULES_IN_PACKET`], the repository's first, then the most
//! trusted. Claude, Codex and OpenCode read the same packet, so they all write
//! code the way the person's reviews ask for. Paths are shown, not filtered:
//! a chat has no changed files yet to match them against.

use bridge_protocol::messages::MemoryRecord;
use rusqlite::Connection;

use crate::BridgeError;

/// Code rules one packet carries.
pub const MAX_TASTE_RULES_IN_PACKET: usize = 8;
/// The kind of rule a coding agent follows.
pub const CODE_KIND: &str = "code_prefer";

/// The code rules a chat is given.
pub fn for_chat(db: &Connection, session_id: &str) -> Result<Vec<MemoryRecord>, BridgeError> {
    let repository = super::repository::scope_of_session(db, session_id)?;
    let account = crate::memory_ledger::account_memory_scope();
    let mut rules: Vec<MemoryRecord> = crate::memory_ledger::list_taste(db, None)?
        .into_iter()
        .filter(|rule| rule.kind == CODE_KIND)
        .filter(|rule| rule.scope_key == account || Some(&rule.scope_key) == repository.as_ref())
        .collect();
    rules.sort_by(|a, b| {
        (b.scope_key != account)
            .cmp(&(a.scope_key != account))
            .then(b.outcome_score.total_cmp(&a.outcome_score))
            .then(b.updated_at.cmp(&a.updated_at))
            .then(a.id.cmp(&b.id))
    });
    rules.truncate(MAX_TASTE_RULES_IN_PACKET);
    Ok(rules)
}

/// The packet's heading over the code rules.
pub const HEADING: &str = "Code rules the person set from their own code reviews, cited by id. Follow them when you \
write or change code in this repository:\n";

/// One rule's line, without its trailing newline.
pub fn line(rule: &MemoryRecord) -> String {
    let short_id: String = rule.id.chars().take(8).collect();
    let paths = rule.path_globs.as_ref().filter(|globs| !globs.is_empty()).map(|globs| format!(" (paths: {})", globs.join(", "))).unwrap_or_default();
    format!("[{short_id}] code rule{paths}: {}", crate::memory_packet::render_body(&rule.body))
}

#[cfg(test)]
#[path = "chat_rules_tests.rs"]
mod tests;

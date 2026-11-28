//! The reviewer follows the person's taste rules (#1202).
//!
//! When a review is prepared, the repository's active rules (and those for
//! every repository) go into the context folder as `taste.json`: at most
//! [`MAX_RULES_PER_REVIEW`], only those whose paths match a changed file or
//! that apply to every path, the path-specific first, then the most trusted,
//! then the most recently changed. The prompt names the file only when it has
//! rules, so a review without taste reads exactly as before. A finding the
//! rules shaped cites their ids; ids the pass was not given are dropped.

use std::path::Path;

use bridge_protocol::messages::MemoryRecord;
use rusqlite::{Connection, Transaction};
use serde_json::{json, Value};

use crate::BridgeError;

/// Rules one review is given, about 2k tokens.
pub const MAX_RULES_PER_REVIEW: usize = 12;
/// Rule text one review is given, in characters.
const MAX_RULE_TEXT: usize = 8000;
/// The context file the rules are in.
pub const TASTE_FILE: &str = "taste.json";

/// Schema 110: which rules shaped a finding, and which a pass was given.
pub(crate) fn install(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    use crate::store::add_column_if_missing;
    add_column_if_missing(transaction, "review_findings", "rule_ids", "TEXT")?;
    add_column_if_missing(transaction, "review_passes", "taste_rule_ids", "TEXT")?;
    Ok(())
}

fn matches_any(globs: &[String], changed: &[String]) -> bool {
    globs.iter().any(|glob| changed.iter().any(|path| crate::policy::glob_matches(glob.trim().trim_start_matches("./"), path)))
}

/// The active rules a review of `scope` with these changed paths is given.
pub fn for_review(db: &Connection, scope: &str, changed: &[String]) -> Result<Vec<MemoryRecord>, BridgeError> {
    let account = crate::memory_ledger::account_memory_scope();
    let mut eligible: Vec<(bool, MemoryRecord)> = crate::memory_ledger::list_taste(db, None)?
        .into_iter()
        .filter(|rule| rule.scope_key == scope || rule.scope_key == account)
        .filter_map(|rule| match &rule.path_globs {
            None => Some((false, rule)),
            Some(globs) if matches_any(globs, changed) => Some((true, rule)),
            Some(_) => None,
        })
        .collect();
    eligible.sort_by(|(a_paths, a), (b_paths, b)| {
        b_paths
            .cmp(a_paths)
            .then(b.outcome_score.total_cmp(&a.outcome_score))
            .then(b.updated_at.cmp(&a.updated_at))
            .then(a.id.cmp(&b.id))
    });
    let mut chosen = Vec::new();
    let mut text = 0;
    for (_, rule) in eligible {
        if chosen.len() == MAX_RULES_PER_REVIEW || text + rule.body.chars().count() > MAX_RULE_TEXT {
            break;
        }
        text += rule.body.chars().count();
        chosen.push(rule);
    }
    Ok(chosen)
}

/// The rules as the reviewer reads them.
pub fn context_json(rules: &[MemoryRecord]) -> Value {
    json!(rules
        .iter()
        .map(|rule| json!({"id": rule.id, "kind": rule.kind, "rule": rule.body, "pathGlobs": rule.path_globs}))
        .collect::<Vec<_>>())
}

/// The rule ids a prepared context gave its reviewer; none when it has no
/// `taste.json` or the file does not read.
pub fn given(context: &Path) -> Vec<String> {
    std::fs::read(context.join(TASTE_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<Value>>(&bytes).ok())
        .unwrap_or_default()
        .iter()
        .filter_map(|rule| rule.get("id").and_then(Value::as_str).map(str::to_owned))
        .collect()
}

/// What the prompt says when a review has rules.
pub fn prompt_line(context: &str) -> String {
    format!(
        "- `{context}/{TASTE_FILE}`: rules the person set for reviews of this repository. Follow them: do not raise \
what a `review_avoid` rule says not to raise unless the change makes it critical; raise what a `review_prefer` rule \
asks for, worded its way; report code that breaks a `code_prefer` rule. On each finding a rule shaped, add \
`\"ruleIds\":[\"<id>\"]` with the ids from that file."
    )
}

#[cfg(test)]
#[path = "review_rules_tests.rs"]
mod tests;

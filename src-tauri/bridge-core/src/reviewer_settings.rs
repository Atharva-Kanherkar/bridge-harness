//! Global settings for the pull-request reviewer worker the GitHub pane
//! launches: which model and effort each harness runs it with, and the
//! instructions it is given.

use crate::{delegation, BridgeError};
use bridge_protocol::messages::{self as wire, ReviewerHarnessSettings, ReviewerSettings};
use rusqlite::{params, Connection, OptionalExtension};

const KIND: &str = "reviewer_settings";
const ID: &str = "global";
const MAX_PROMPT_CHARS: usize = 20_000;
const HARNESSES: [&str; 3] = ["claude", "codex", "opencode"];

/// What the reviewer is told when the user has not written their own prompt.
/// `{number}` is the pull request number.
pub const DEFAULT_SYSTEM_PROMPT: &str = "Review pull request #{number} in this repository. Run `gh pr view {number}` and \
`gh pr diff {number}` to read the change, then post a concise, constructive code \
review as a comment using `gh pr comment {number} --body \"...\"`. Cite concrete \
files and line numbers; call out correctness bugs, risky changes, and missing tests. \
Do NOT approve, merge, request-changes, or close the PR — only post a comment.";

pub fn load(db: &Connection) -> Result<ReviewerSettings, BridgeError> {
    let payload: Option<String> = db
        .query_row(
            "SELECT payload FROM configuration_entries WHERE kind=?1 AND id=?2",
            params![KIND, ID],
            |row| row.get(0),
        )
        .optional()?;
    payload
        .map(|payload| serde_json::from_str(&payload).map_err(|error| BridgeError::Invalid(error.to_string())))
        .unwrap_or_else(|| Ok(ReviewerSettings::default()))
}

pub fn save(db: &Connection, settings: &ReviewerSettings) -> Result<ReviewerSettings, BridgeError> {
    if let Some(unknown) = settings.harnesses.keys().find(|id| !HARNESSES.contains(&id.as_str())) {
        return Err(BridgeError::Invalid(format!(
            "Unknown reviewer harness `{unknown}`; expected one of claude, codex, opencode"
        )));
    }
    if settings.system_prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err(BridgeError::Invalid(format!(
            "The reviewer prompt is longer than {MAX_PROMPT_CHARS} characters"
        )));
    }
    // Store what means something: a blank model or prompt is "unset", not "".
    let mut normalized = settings.clone();
    normalized.system_prompt = normalized.system_prompt.trim().to_owned();
    for entry in normalized.harnesses.values_mut() {
        if entry.model.as_deref().is_some_and(|model| model.trim().is_empty()) {
            entry.model = None;
        }
    }
    normalized
        .harnesses
        .retain(|_, entry| *entry != ReviewerHarnessSettings::default());
    db.execute(
        "INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at)
         VALUES(?1,?2,?3,?4,?4) ON CONFLICT(kind,id) DO UPDATE SET payload=excluded.payload,updated_at=excluded.updated_at",
        params![
            KIND,
            ID,
            serde_json::to_string(&normalized).map_err(|error| BridgeError::Invalid(error.to_string()))?,
            chrono::Utc::now().to_rfc3339()
        ],
    )?;
    load(db)
}

pub fn view(settings: ReviewerSettings) -> wire::ReviewerSettingsResult {
    wire::ReviewerSettingsResult {
        settings,
        default_system_prompt: DEFAULT_SYSTEM_PROMPT.into(),
    }
}

/// The objective the review worker receives: the user's prompt when they
/// wrote one, else the default, with the PR number filled in either way.
pub fn objective(settings: &ReviewerSettings, number: u64) -> String {
    let template = if settings.system_prompt.trim().is_empty() {
        DEFAULT_SYSTEM_PROMPT
    } else {
        settings.system_prompt.trim()
    };
    template.replace("{number}", &number.to_string())
}

pub fn effort_from_wire(effort: wire::Effort) -> delegation::Effort {
    match effort {
        wire::Effort::Low => delegation::Effort::Low,
        wire::Effort::Medium => delegation::Effort::Medium,
        wire::Effort::High => delegation::Effort::High,
        wire::Effort::Xhigh => delegation::Effort::Xhigh,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> (tempfile::TempDir, Connection) {
        let scratch = tempfile::tempdir().unwrap();
        let db = crate::store::open(&scratch.path().join("test.db")).unwrap();
        (scratch, db)
    }

    #[test]
    fn round_trips_and_defaults() {
        let (_scratch, db) = db();
        assert_eq!(load(&db).unwrap(), ReviewerSettings::default());
        let mut settings = ReviewerSettings::default();
        settings.harnesses.insert(
            "codex".into(),
            ReviewerHarnessSettings { model: Some("gpt-5-codex".into()), effort: Some(wire::Effort::Xhigh) },
        );
        // A blank model and an all-default harness entry are noise, not settings.
        settings.harnesses.insert("claude".into(), ReviewerHarnessSettings { model: Some("  ".into()), effort: None });
        settings.system_prompt = "  Review #{number} for security only.  ".into();
        let stored = save(&db, &settings).unwrap();
        assert_eq!(stored.harnesses.len(), 1);
        assert_eq!(stored.harnesses["codex"].model.as_deref(), Some("gpt-5-codex"));
        assert_eq!(stored.system_prompt, "Review #{number} for security only.");
        assert_eq!(load(&db).unwrap(), stored);
    }

    #[test]
    fn rejects_unknown_harnesses_and_oversized_prompts() {
        let (_scratch, db) = db();
        let mut unknown = ReviewerSettings::default();
        unknown.harnesses.insert("cursor".into(), ReviewerHarnessSettings::default());
        assert!(save(&db, &unknown).unwrap_err().to_string().contains("Unknown reviewer harness"));
        let long = ReviewerSettings { system_prompt: "x".repeat(MAX_PROMPT_CHARS + 1), ..Default::default() };
        assert!(save(&db, &long).unwrap_err().to_string().contains("longer than"));
        assert_eq!(load(&db).unwrap(), ReviewerSettings::default(), "a refused save stores nothing");
    }

    #[test]
    fn objective_uses_the_default_or_the_custom_prompt_with_the_number_expanded() {
        let default = objective(&ReviewerSettings::default(), 42);
        assert!(default.starts_with("Review pull request #42"));
        assert!(default.contains("gh pr diff 42"));
        assert!(!default.contains("{number}"));
        let custom = ReviewerSettings { system_prompt: "Only check PR {number} for tests.".into(), ..Default::default() };
        assert_eq!(objective(&custom, 7), "Only check PR 7 for tests.");
        let blank = ReviewerSettings { system_prompt: "   ".into(), ..Default::default() };
        assert_eq!(objective(&blank, 42), default, "whitespace is not a prompt");
    }
}

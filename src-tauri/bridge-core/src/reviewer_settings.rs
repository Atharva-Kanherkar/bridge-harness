//! Global settings for the pull-request reviewer worker the GitHub pane
//! launches: which model and effort each harness runs it with, and the
//! instructions it is given.

use crate::{delegation, BridgeError};
use bridge_protocol::messages::{self as wire, ReviewerHarnessSettings, ReviewerSettings};
use rusqlite::{params, Connection, OptionalExtension};

const KIND: &str = "reviewer_settings";
const ID: &str = "global";
const MAX_PROMPT_CHARS: usize = 20_000;

/// The instruction a custom reviewer prompt must never drop: the reviewer
/// posts one comment and takes no other PR action. A custom prompt replaces
/// the default wholesale, so `objective` re-appends this when it is missing.
pub const SAFETY_GUARDRAIL: &str =
    "Do NOT approve, merge, request-changes, or close the PR — only post a comment.";

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

/// Canonical harness id for a reviewer-settings key: the shared delegation
/// normalizer, so aliases (`claude-code`, `open-code`, …) fold to the id the
/// launch path resolves. Cursor Bugbot is not a local worker but
/// `github_review` accepts it, so its aliases fold to `cursor-bugbot` rather
/// than erroring as unknown.
pub fn canonical_harness(id: &str) -> Option<String> {
    let lower = id.trim().to_ascii_lowercase();
    if matches!(lower.as_str(), "bugbot" | "cursor-bugbot" | "cursor_bugbot") {
        return Some("cursor-bugbot".into());
    }
    crate::delegation::normalize_harness(id)
}

pub fn save(db: &Connection, settings: &ReviewerSettings) -> Result<ReviewerSettings, BridgeError> {
    let mut canonical: std::collections::BTreeMap<String, ReviewerHarnessSettings> =
        std::collections::BTreeMap::new();
    for (id, entry) in &settings.harnesses {
        match canonical_harness(id) {
            Some(canonical_id) => {
                canonical
                    .entry(canonical_id)
                    .and_modify(|existing: &mut ReviewerHarnessSettings| {
                        // Two aliases for one harness: prefer an explicitly set
                        // model/effort over a default.
                        if existing.model.is_none() {
                            existing.model = entry.model.clone();
                        }
                        if existing.effort.is_none() {
                            existing.effort = entry.effort;
                        }
                    })
                    .or_insert_with(|| entry.clone());
            }
            None => {
                return Err(BridgeError::Invalid(format!(
                    "Unknown reviewer harness `{id}`"
                )));
            }
        }
    }
    if settings.system_prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err(BridgeError::Invalid(format!(
            "The reviewer prompt is longer than {MAX_PROMPT_CHARS} characters"
        )));
    }
    if !wire::MAX_CONCURRENT_REVIEWS_RANGE.contains(&settings.max_concurrent_reviews) {
        return Err(BridgeError::Invalid(format!(
            "Reviews at once must be between {} and {}",
            wire::MAX_CONCURRENT_REVIEWS_RANGE.start(),
            wire::MAX_CONCURRENT_REVIEWS_RANGE.end()
        )));
    }
    let default_harnesses = review_job_harnesses(&settings.default_harnesses, "default reviewer")?;
    if default_harnesses.len() > MAX_REVIEW_HARNESSES {
        return Err(BridgeError::Invalid(format!(
            "A review runs on at most {MAX_REVIEW_HARNESSES} harnesses"
        )));
    }
    let fallback_harnesses = review_job_harnesses(&settings.fallback_harnesses, "fallback reviewer")?;
    // Store what means something: a blank model or prompt is "unset", not "".
    let mut normalized = settings.clone();
    normalized.system_prompt = normalized.system_prompt.trim().to_owned();
    normalized.harnesses = canonical;
    normalized.default_harnesses = default_harnesses;
    normalized.fallback_harnesses = fallback_harnesses;
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

/// Reviewers one job may ask for: one, or two for a second opinion.
pub const MAX_REVIEW_HARNESSES: usize = 2;

/// A list of harnesses a review job may run on, canonicalized and without
/// repeats. Cursor Bugbot is refused: it reviews on GitHub, not as a pass.
/// `label` names the list in errors: "reviewer", "default reviewer", ...
pub fn review_job_harnesses(ids: &[String], label: &str) -> Result<Vec<String>, BridgeError> {
    let mut harnesses: Vec<String> = Vec::new();
    for id in ids {
        if id.trim().is_empty() {
            continue;
        }
        let canonical = match canonical_harness(id) {
            Some(canonical) if canonical != "cursor-bugbot" => canonical,
            Some(_) => {
                return Err(BridgeError::Invalid(format!(
                    "Cursor Bugbot cannot be a {label} harness: it reviews on GitHub"
                )))
            }
            None => return Err(BridgeError::Invalid(format!("Unknown {label} harness `{id}`"))),
        };
        if !harnesses.contains(&canonical) {
            harnesses.push(canonical);
        }
    }
    Ok(harnesses)
}

pub fn view(settings: ReviewerSettings) -> wire::ReviewerSettingsResult {
    wire::ReviewerSettingsResult {
        settings,
        default_system_prompt: DEFAULT_SYSTEM_PROMPT.into(),
    }
}

/// The objective the review worker receives: the user's prompt when they
/// wrote one, else the default, with the PR number filled in either way. A
/// custom prompt replaces the default wholesale, so the mandatory
/// no-approve/no-merge guardrail is re-appended when the custom text drops it.
pub fn objective(settings: &ReviewerSettings, number: u64) -> String {
    objective_with_hide(settings, number, false)
}

/// Same as `objective`, but prepends the strict no-attribution rule first when
/// hiding is ON.
pub fn objective_with_hide(settings: &ReviewerSettings, number: u64, hide: bool) -> String {
    let template = if settings.system_prompt.trim().is_empty() {
        DEFAULT_SYSTEM_PROMPT
    } else {
        settings.system_prompt.trim()
    };
    let expanded = template.replace("{number}", &number.to_string());
    let guarded = if settings.system_prompt.trim().is_empty() {
        expanded
    } else if expanded.to_ascii_lowercase().contains("do not approve") {
        expanded
    } else {
        format!("{expanded}\n\n{SAFETY_GUARDRAIL}")
    };
    if hide {
        format!("{}\n\n{}", crate::prompts::ATTRIBUTION_HIDING_RULE, guarded)
    } else {
        guarded
    }
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
        // `shell` is a valid harness id that delegation refuses on purpose;
        // the reviewer has no shell review path either.
        unknown.harnesses.insert("shell".into(), ReviewerHarnessSettings::default());
        assert!(save(&db, &unknown).unwrap_err().to_string().contains("Unknown reviewer harness"));
        let long = ReviewerSettings { system_prompt: "x".repeat(MAX_PROMPT_CHARS + 1), ..Default::default() };
        assert!(save(&db, &long).unwrap_err().to_string().contains("longer than"));
        assert_eq!(load(&db).unwrap(), ReviewerSettings::default(), "a refused save stores nothing");
    }

    #[test]
    fn harness_keys_are_canonicalized_through_the_shared_normalizer() {
        let (_scratch, db) = db();
        let mut settings = ReviewerSettings::default();
        settings.harnesses.insert(
            "claude-code".into(),
            ReviewerHarnessSettings { model: Some("m".into()), effort: None },
        );
        settings.harnesses.insert(
            "bugbot".into(),
            ReviewerHarnessSettings { model: None, effort: Some(wire::Effort::High) },
        );
        let stored = save(&db, &settings).unwrap();
        assert!(stored.harnesses.contains_key("claude"), "aliases fold to the canonical id");
        assert!(stored.harnesses.contains_key("cursor-bugbot"), "the bugbot github_review accepts is not rejected");
        assert!(!stored.harnesses.contains_key("claude-code"));
        assert!(!stored.harnesses.contains_key("bugbot"));
    }

    #[test]
    fn runner_settings_default_and_validate() {
        let (_scratch, db) = db();
        let defaults = load(&db).unwrap();
        assert_eq!(defaults.max_concurrent_reviews, 2);
        assert_eq!(defaults.post_mode, wire::ReviewPostMode::Draft);
        assert!(defaults.default_harnesses.is_empty() && defaults.fallback_harnesses.is_empty());

        for refused in [0, 5] {
            let settings = ReviewerSettings { max_concurrent_reviews: refused, ..Default::default() };
            assert!(save(&db, &settings).unwrap_err().to_string().contains("between 1 and 4"), "{refused}");
        }
        let settings = ReviewerSettings {
            default_harnesses: vec!["claude-code".into(), "codex".into(), "claude".into()],
            fallback_harnesses: vec!["open-code".into(), " ".into(), "codex".into()],
            max_concurrent_reviews: 4,
            post_mode: wire::ReviewPostMode::Auto,
            ..Default::default()
        };
        let stored = save(&db, &settings).unwrap();
        assert_eq!(stored.default_harnesses, ["claude", "codex"], "aliases fold and repeats drop");
        assert_eq!(stored.fallback_harnesses, ["opencode", "codex"]);
        assert_eq!(stored.max_concurrent_reviews, 4);
        assert_eq!(stored.post_mode, wire::ReviewPostMode::Auto);

        let three = ReviewerSettings { default_harnesses: vec!["claude".into(), "codex".into(), "opencode".into()], ..Default::default() };
        assert!(save(&db, &three).unwrap_err().to_string().contains("at most 2"));
        let bugbot = ReviewerSettings { fallback_harnesses: vec!["bugbot".into()], ..Default::default() };
        assert!(save(&db, &bugbot).unwrap_err().to_string().contains("Cursor Bugbot"));
        let unknown = ReviewerSettings { default_harnesses: vec!["shell".into()], ..Default::default() };
        assert!(save(&db, &unknown).unwrap_err().to_string().contains("Unknown default reviewer harness"));
        assert_eq!(load(&db).unwrap(), stored, "a refused save stores nothing");
    }

    #[test]
    fn a_payload_stored_before_runner_settings_loads_with_defaults() {
        let (_scratch, db) = db();
        db.execute(
            "INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at) VALUES(?1,?2,?3,'now','now')",
            params![KIND, ID, r#"{"harnesses":{"codex":{"model":"gpt-5-codex"}},"systemPrompt":"Review #{number}."}"#],
        )
        .unwrap();
        let loaded = load(&db).unwrap();
        assert_eq!(loaded.harnesses["codex"].model.as_deref(), Some("gpt-5-codex"));
        assert_eq!(loaded.system_prompt, "Review #{number}.");
        assert_eq!(loaded.max_concurrent_reviews, 2);
        assert_eq!(loaded.post_mode, wire::ReviewPostMode::Draft);
        assert!(loaded.default_harnesses.is_empty() && loaded.fallback_harnesses.is_empty());
    }

    #[test]
    fn custom_objective_retains_the_safety_guardrail() {
        let custom = ReviewerSettings { system_prompt: "Only check PR {number} for tests.".into(), ..Default::default() };
        let rendered = objective(&custom, 7);
        assert!(rendered.starts_with("Only check PR 7 for tests."));
        assert!(rendered.contains("only post a comment"), "the no-approve/no-merge guardrail survives a custom prompt: {rendered}");
        let already_safe = ReviewerSettings { system_prompt: "Check {number}. Do NOT approve anything.".into(), ..Default::default() };
        assert_eq!(objective(&already_safe, 7).matches("Do NOT approve").count(), 1, "a prompt that already guards is not doubled");
    }

    #[test]
    fn objective_uses_the_default_or_the_custom_prompt_with_the_number_expanded() {
        let default = objective(&ReviewerSettings::default(), 42);
        assert!(default.starts_with("Review pull request #42"));
        assert!(default.contains("gh pr diff 42"));
        assert!(!default.contains("{number}"));
        let blank = ReviewerSettings { system_prompt: "   ".into(), ..Default::default() };
        assert_eq!(objective(&blank, 42), default, "whitespace is not a prompt");
    }

    #[test]
    fn objective_with_hide_prepends_strict_rule_first() {
        let hidden = objective_with_hide(&ReviewerSettings::default(), 7, true);
        assert!(hidden.starts_with(crate::prompts::ATTRIBUTION_HIDING_RULE));
        assert!(hidden.contains("Review pull request #7"));
        let shown = objective_with_hide(&ReviewerSettings::default(), 7, false);
        assert_eq!(shown, objective(&ReviewerSettings::default(), 7));
        assert!(!shown.starts_with(crate::prompts::ATTRIBUTION_HIDING_RULE));
    }
}

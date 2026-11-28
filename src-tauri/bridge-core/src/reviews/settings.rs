//! How review jobs run: default and fallback harnesses, the runner's pool
//! size, and whether findings wait for triage. One record for the account,
//! stored apart from the reviewer's model and prompt (`reviewer_settings`) so
//! saving either never touches the other.

use bridge_protocol::messages::{ReviewJobSettings, MAX_CONCURRENT_REVIEWS_RANGE, MAX_REVIEW_WORKTREES_RANGE};
use rusqlite::{params, Connection, OptionalExtension};

use crate::BridgeError;

const KIND: &str = "review_job_settings";
const ID: &str = "global";

/// Reviewers one job may ask for: one, or two for a second opinion.
pub const MAX_REVIEW_HARNESSES: usize = 2;

pub fn load(db: &Connection) -> Result<ReviewJobSettings, BridgeError> {
    let payload: Option<String> = db
        .query_row(
            "SELECT payload FROM configuration_entries WHERE kind=?1 AND id=?2",
            params![KIND, ID],
            |row| row.get(0),
        )
        .optional()?;
    let mut settings: ReviewJobSettings = payload
        .map(|payload| serde_json::from_str(&payload).map_err(|error| BridgeError::Invalid(error.to_string())))
        .unwrap_or_else(|| Ok(ReviewJobSettings::default()))?;
    // A pair of defaults stored before `second_opinion` existed was valid
    // then and meant two reviewers. `save` refuses a new pair with it off, so
    // only such a record has both: it loads as it ran.
    if settings.default_harnesses.len() > 1 && !settings.second_opinion {
        settings.second_opinion = true;
    }
    Ok(settings)
}

pub fn save(db: &Connection, settings: &ReviewJobSettings) -> Result<ReviewJobSettings, BridgeError> {
    if !MAX_CONCURRENT_REVIEWS_RANGE.contains(&settings.max_concurrent_reviews) {
        return Err(BridgeError::Invalid(format!(
            "Reviews at once must be between {} and {}",
            MAX_CONCURRENT_REVIEWS_RANGE.start(),
            MAX_CONCURRENT_REVIEWS_RANGE.end()
        )));
    }
    if !MAX_REVIEW_WORKTREES_RANGE.contains(&settings.max_review_worktrees) {
        return Err(BridgeError::Invalid(format!(
            "Review checkouts must be between {} and {}",
            MAX_REVIEW_WORKTREES_RANGE.start(),
            MAX_REVIEW_WORKTREES_RANGE.end()
        )));
    }
    let default_harnesses = harnesses(&settings.default_harnesses, "default reviewer")?;
    if default_harnesses.len() > MAX_REVIEW_HARNESSES {
        return Err(BridgeError::Invalid(format!("A review runs on at most {MAX_REVIEW_HARNESSES} harnesses")));
    }
    if default_harnesses.len() > 1 && !settings.second_opinion {
        return Err(BridgeError::Invalid(
            "Turn on Second opinion to review with two harnesses by default".into(),
        ));
    }
    let normalized = ReviewJobSettings {
        default_harnesses,
        fallback_harnesses: harnesses(&settings.fallback_harnesses, "fallback reviewer")?,
        ..settings.clone()
    };
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

/// A list of harnesses a review job may run on, canonicalized and without
/// repeats. Cursor Bugbot is refused: it reviews on GitHub, not as a pass.
/// `label` names the list in errors ("reviewer", "default reviewer", ...).
pub fn harnesses(ids: &[String], label: &str) -> Result<Vec<String>, BridgeError> {
    let mut harnesses: Vec<String> = Vec::new();
    for id in ids {
        if id.trim().is_empty() {
            continue;
        }
        let canonical = match crate::reviewer_settings::canonical_harness(id) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use bridge_protocol::messages::{self as wire, ReviewerHarnessSettings, ReviewerSettings};

    fn db() -> (tempfile::TempDir, Connection) {
        let scratch = tempfile::tempdir().unwrap();
        let db = crate::store::open(&scratch.path().join("test.db")).unwrap();
        (scratch, db)
    }

    #[test]
    fn runner_settings_default_and_validate() {
        let (_scratch, db) = db();
        let defaults = load(&db).unwrap();
        assert_eq!(defaults, ReviewJobSettings::default());
        assert_eq!(defaults.max_concurrent_reviews, 2);
        assert_eq!(defaults.post_mode, wire::ReviewPostMode::Draft);

        for refused in [0, 5] {
            let settings = ReviewJobSettings { max_concurrent_reviews: refused, ..Default::default() };
            assert!(save(&db, &settings).unwrap_err().to_string().contains("between 1 and 4"), "{refused}");
        }
        let settings = ReviewJobSettings {
            default_harnesses: vec!["claude-code".into(), "codex".into(), "claude".into()],
            fallback_harnesses: vec!["open-code".into(), " ".into(), "codex".into()],
            max_concurrent_reviews: 4,
            post_mode: wire::ReviewPostMode::Auto,
            max_review_worktrees: 8,
            second_opinion: true,
            ..Default::default()
        };
        let stored = save(&db, &settings).unwrap();
        assert_eq!(stored.default_harnesses, ["claude", "codex"], "aliases fold and repeats drop");
        assert_eq!(stored.fallback_harnesses, ["opencode", "codex"]);
        assert_eq!(stored.max_concurrent_reviews, 4);
        assert_eq!(stored.post_mode, wire::ReviewPostMode::Auto);

        let three = ReviewJobSettings {
            default_harnesses: vec!["claude".into(), "codex".into(), "opencode".into()],
            second_opinion: true,
            ..Default::default()
        };
        assert!(save(&db, &three).unwrap_err().to_string().contains("at most 2"));
        let bugbot = ReviewJobSettings { fallback_harnesses: vec!["bugbot".into()], ..Default::default() };
        assert!(save(&db, &bugbot).unwrap_err().to_string().contains("Cursor Bugbot"));
        let unknown = ReviewJobSettings { default_harnesses: vec!["shell".into()], ..Default::default() };
        assert!(save(&db, &unknown).unwrap_err().to_string().contains("Unknown default reviewer harness"));
        assert_eq!(load(&db).unwrap(), stored, "a refused save stores nothing");
    }

    #[test]
    fn two_default_harnesses_need_second_opinion() {
        let (_scratch, db) = db();
        let two = ReviewJobSettings { default_harnesses: vec!["claude".into(), "codex".into()], ..Default::default() };
        assert!(save(&db, &two).unwrap_err().to_string().contains("Turn on Second opinion"));
        assert_eq!(load(&db).unwrap(), ReviewJobSettings::default(), "nothing stored");
        let on = save(&db, &ReviewJobSettings { second_opinion: true, ..two }).unwrap();
        assert!(on.second_opinion);
        assert_eq!(on.default_harnesses, ["claude", "codex"]);
        let one = ReviewJobSettings { default_harnesses: vec!["codex".into()], ..Default::default() };
        assert_eq!(save(&db, &one).unwrap().default_harnesses, ["codex"], "one default needs nothing");
    }

    #[test]
    fn a_stored_pair_from_before_second_opinion_still_loads_and_saves() {
        let (_scratch, db) = db();
        save(&db, &ReviewJobSettings::default()).unwrap();
        // Valid when it was stored: the field did not exist yet.
        db.execute(
            "UPDATE configuration_entries SET payload=?1 WHERE kind=?2",
            params![r#"{"defaultHarnesses":["claude","codex"],"maxConcurrentReviews":2}"#, KIND],
        )
        .unwrap();
        let loaded = load(&db).unwrap();
        assert!(loaded.second_opinion, "the pair meant two reviewers");
        assert_eq!(loaded.default_harnesses, ["claude", "codex"]);
        // What the form sends when the person changes something unrelated.
        let saved = save(&db, &ReviewJobSettings { max_concurrent_reviews: 3, ..loaded }).unwrap();
        assert_eq!(saved.max_concurrent_reviews, 3);
        assert!(saved.second_opinion);
    }

    #[test]
    fn auto_review_and_incremental_are_off_by_default_and_round_trip() {
        let (_scratch, db) = db();
        let defaults = load(&db).unwrap();
        assert_eq!(defaults.auto_review, wire::ReviewAutoMode::Off);
        assert!(defaults.skip_draft_prs, "drafts are skipped unless asked");
        assert!(!defaults.incremental);
        let on = ReviewJobSettings { auto_review: wire::ReviewAutoMode::Mine, skip_draft_prs: false, incremental: true, ..Default::default() };
        assert_eq!(save(&db, &on).unwrap(), on);
        assert_eq!(load(&db).unwrap(), on);
        // A record from before these fields loads them off.
        db.execute("UPDATE configuration_entries SET payload=?1 WHERE kind=?2", params![r#"{"postMode":"auto"}"#, KIND]).unwrap();
        let old = load(&db).unwrap();
        assert_eq!((old.auto_review, old.skip_draft_prs, old.incremental), (wire::ReviewAutoMode::Off, true, false));
        assert_eq!(old.post_mode, wire::ReviewPostMode::Auto);
    }

    #[test]
    fn max_review_worktrees_defaults_to_eight_and_is_bounded() {
        let (_scratch, db) = db();
        assert_eq!(load(&db).unwrap().max_review_worktrees, 8);
        for refused in [0, 33] {
            let settings = ReviewJobSettings { max_review_worktrees: refused, ..Default::default() };
            assert!(save(&db, &settings).unwrap_err().to_string().contains("between 1 and 32"), "{refused}");
        }
        let saved = save(&db, &ReviewJobSettings { max_review_worktrees: 32, ..Default::default() }).unwrap();
        assert_eq!(saved.max_review_worktrees, 32);
        db.execute(
            "UPDATE configuration_entries SET payload=?1 WHERE kind=?2",
            params![r#"{"postMode":"draft"}"#, KIND],
        )
        .unwrap();
        assert_eq!(load(&db).unwrap().max_review_worktrees, 8, "a record from before the field loads the default");
    }

    #[test]
    fn saving_reviewer_settings_leaves_review_job_settings_alone() {
        let (_scratch, db) = db();
        let jobs = save(
            &db,
            &ReviewJobSettings {
                default_harnesses: vec!["codex".into()],
                fallback_harnesses: vec!["claude".into()],
                max_concurrent_reviews: 1,
                post_mode: wire::ReviewPostMode::Auto,
                max_review_worktrees: 3,
                second_opinion: false,
                auto_review: wire::ReviewAutoMode::Mine,
                skip_draft_prs: false,
                incremental: true,
                handoff: wire::ReviewHandoff::Posted,
                hide_repeat_drops: false,
                follow_taste_rules: false,
            },
        )
        .unwrap();
        // What the existing reviewer form sends: only the two fields it edits.
        let mut reviewer = ReviewerSettings { system_prompt: "Only security, PR {number}.".into(), ..Default::default() };
        reviewer
            .harnesses
            .insert("codex".into(), ReviewerHarnessSettings { model: Some("gpt-5-codex".into()), effort: None });
        crate::reviewer_settings::save(&db, &reviewer).unwrap();
        assert_eq!(load(&db).unwrap(), jobs, "a reviewer model or prompt save never resets job settings");
        save(&db, &ReviewJobSettings::default()).unwrap();
        assert_eq!(crate::reviewer_settings::load(&db).unwrap().system_prompt, "Only security, PR {number}.");
    }

    #[test]
    fn a_record_with_fields_from_a_newer_build_still_loads() {
        let (_scratch, db) = db();
        db.execute(
            "INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at) VALUES(?1,?2,?3,'now','now')",
            params![KIND, ID, r#"{"postMode":"auto","autoReview":"mine"}"#],
        )
        .unwrap();
        let loaded = load(&db).unwrap();
        assert_eq!(loaded.post_mode, wire::ReviewPostMode::Auto);
        assert_eq!(loaded.max_concurrent_reviews, 2, "a field it lacks takes its default");
    }
}

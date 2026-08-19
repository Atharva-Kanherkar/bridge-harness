//! The Work board: what needs doing, read from SQLite and nothing else.
//!
//! [`board`] is the whole read path behind `work/get_work_board`, and it is
//! **store-only** by construction: it takes a `&Connection`, not a
//! `&Arc<BridgeCore>`, so it cannot reach an adapter map, a connector, or a
//! process spawner even by accident. Anything the board needs that SQLite does
//! not already hold is observed elsewhere and served from `work_fact_cache`
//! with the timestamp of that observation.
//!
//! The DTOs are `bridge_protocol::messages`' Work types used directly. A second
//! copy in this crate would only create something to drift.

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension};

use bridge_protocol::messages as wire;

use crate::BridgeError;

/// The defaults a board reports when Work has never been configured. Chosen to
/// match the epic's stated operational bounds: a 10-minute run deadline inside
/// a 15-minute lease, and a cooldown that cannot be shorter than the lease.
pub const DEFAULT_MAX_WALL_SECONDS: i64 = 600;
pub const DEFAULT_MAX_TURNS: i64 = 12;
pub const DEFAULT_MAX_TOOL_CALLS: i64 = 24;
pub const DEFAULT_COOLDOWN_MINUTES: i64 = 15;

/// Where Work's configuration lives in `configuration_entries`.
const SETTINGS_KIND: &str = "work";
const SETTINGS_ID: &str = "settings";

/// Work's configuration before anyone has configured it: facts only, no model,
/// no connector, no cadence.
pub fn default_settings() -> wire::WorkSettings {
    wire::WorkSettings {
        briefing: None,
        enabled_connector_instances: Vec::new(),
        refresh_on_focus: false,
        refresh_interval_minutes: None,
        cooldown_minutes: DEFAULT_COOLDOWN_MINUTES,
        limits: wire::WorkBriefLimits {
            max_wall_seconds: DEFAULT_MAX_WALL_SECONDS,
            max_turns: DEFAULT_MAX_TURNS,
            max_tool_calls: DEFAULT_MAX_TOOL_CALLS,
            max_output_tokens: None,
            cost_ceiling_microusd: None,
        },
    }
}

/// Read Work's settings. `Ok(None)` means never configured; `Err` means a row
/// exists that this contract cannot read, which the caller degrades rather than
/// failing the whole board over.
fn stored_settings(db: &Connection) -> Result<Option<wire::WorkSettings>, BridgeError> {
    // `optional()`, not `ok()`: an absent row means "never configured", while a
    // database error means the read failed and must not read as the same thing.
    let payload: Option<String> = db
        .query_row(
            "SELECT payload FROM configuration_entries WHERE kind=?1 AND id=?2",
            rusqlite::params![SETTINGS_KIND, SETTINGS_ID],
            |row| row.get(0),
        )
        .optional()?;
    let Some(payload) = payload else {
        return Ok(None);
    };
    serde_json::from_str(&payload)
        .map(Some)
        .map_err(|error| BridgeError::Invalid(format!("stored Work settings are invalid: {error}")))
}

/// The board. Deterministic, offline, and useful with no model configured.
pub fn board(db: &Connection) -> Result<wire::WorkBoard, BridgeError> {
    let (settings, settings_error) = match stored_settings(db) {
        Ok(Some(settings)) => (settings, None),
        Ok(None) => (default_settings(), None),
        // A configuration row we cannot read is worth saying out loud, but it
        // is not worth withholding every fact over: the facts do not depend on
        // it. Fall back to defaults and let `suggestions` carry the reason.
        Err(_) => (
            default_settings(),
            Some("stored Work settings could not be read; using defaults".to_owned()),
        ),
    };

    // Suggested work is contracted but not yet produced by anything. Until the
    // briefing runner lands, the honest state is "no model is configured" —
    // never an empty list that looks like "nothing to suggest".
    let suggestions = match settings_error {
        Some(detail) => wire::WorkSuggestions {
            state: wire::WorkSuggestionsState::Degraded,
            detail: Some(detail),
        },
        None => wire::WorkSuggestions {
            state: wire::WorkSuggestionsState::NotConfigured,
            detail: Some(
                "no briefing model is configured, so Work is showing facts only".to_owned(),
            ),
        },
    };

    Ok(wire::WorkBoard {
        generated_at: Utc::now().to_rfc3339(),
        facts: Vec::new(),
        tasks: Vec::new(),
        latest_run: None,
        sources: Vec::new(),
        usage: None,
        settings,
        suggestions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;

    fn memory_db() -> Connection {
        store::open(std::path::Path::new(":memory:")).unwrap()
    }

    fn store_settings(db: &Connection, payload: &str) {
        db.execute(
            "INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at)
             VALUES(?1,?2,?3,'now','now')",
            rusqlite::params![SETTINGS_KIND, SETTINGS_ID, payload],
        )
        .unwrap();
    }

    #[test]
    fn absent_settings_read_as_the_documented_defaults() {
        let db = memory_db();
        let board = board(&db).unwrap();
        assert_eq!(board.settings, default_settings());
        assert!(board.settings.briefing.is_none());
        assert_eq!(board.settings.limits.max_wall_seconds, DEFAULT_MAX_WALL_SECONDS);
        assert_eq!(board.suggestions.state, wire::WorkSuggestionsState::NotConfigured);
    }

    #[test]
    fn a_board_with_no_model_or_connector_is_still_a_board() {
        let db = memory_db();
        let board = board(&db).unwrap();
        assert!(board.tasks.is_empty(), "no briefing runner has produced tasks");
        assert!(board.latest_run.is_none());
        assert!(board.sources.is_empty());
        assert!(board.usage.is_none());
        assert!(!board.generated_at.is_empty());
    }

    #[test]
    fn configured_settings_are_read_back_verbatim() {
        let db = memory_db();
        store_settings(
            &db,
            r#"{"briefing":{"harness":"claude","model":"claude-opus-5","effort":"high"},
                "enabledConnectorInstances":["github:acme"],"refreshOnFocus":true,
                "refreshIntervalMinutes":30,"cooldownMinutes":20,
                "limits":{"maxWallSeconds":300,"maxTurns":6,"maxToolCalls":12,
                          "maxOutputTokens":null,"costCeilingMicrousd":null}}"#,
        );
        let board = board(&db).unwrap();
        let briefing = board.settings.briefing.as_ref().expect("briefing profile");
        assert_eq!(briefing.harness.as_str(), "claude");
        assert_eq!(briefing.model, "claude-opus-5");
        assert_eq!(board.settings.enabled_connector_instances, vec!["github:acme".to_owned()]);
        assert_eq!(board.settings.cooldown_minutes, 20);
        assert_eq!(board.settings.limits.max_turns, 6);
    }

    #[test]
    fn settings_that_cannot_be_read_degrade_instead_of_failing_the_board() {
        let db = memory_db();
        // An unknown field is exactly what a rolled-back binary would meet.
        store_settings(
            &db,
            r#"{"briefing":null,"enabledConnectorInstances":[],"refreshOnFocus":false,
                "refreshIntervalMinutes":null,"cooldownMinutes":15,
                "limits":{"maxWallSeconds":600,"maxTurns":12,"maxToolCalls":24,
                          "maxOutputTokens":null,"costCeilingMicrousd":null},
                "writeConnectorTools":true}"#,
        );
        let board = board(&db).unwrap();
        assert_eq!(board.settings, default_settings(), "defaults, not a half-read payload");
        assert_eq!(board.suggestions.state, wire::WorkSuggestionsState::Degraded);
        assert!(board
            .suggestions
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("could not be read")));
    }
}

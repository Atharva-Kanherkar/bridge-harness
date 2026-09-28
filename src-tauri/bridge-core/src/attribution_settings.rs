//! Global toggle for hiding AI attribution in model-generated git and GitHub text.
//!
//! When ON, Bridge prepends a strict no-attribution rule as the first prompt
//! content for every target, so models never add Co-authored-by, Generated-by,
//! or harness mentions to commits, PR descriptions, or comments. When OFF
//! (the default), prompts are unchanged.

use crate::BridgeError;
use bridge_protocol::messages::{AttributionSettings, SaveAttributionSettingsParams};
use rusqlite::{params, Connection, OptionalExtension};

const KIND: &str = "attribution_settings";
const ID: &str = "global";

pub fn load(db: &Connection) -> Result<AttributionSettings, BridgeError> {
    let payload: Option<String> = db
        .query_row(
            "SELECT payload FROM configuration_entries WHERE kind=?1 AND id=?2",
            params![KIND, ID],
            |row| row.get(0),
        )
        .optional()?;
    payload
        .map(|payload| {
            serde_json::from_str(&payload).map_err(|error| BridgeError::Invalid(error.to_string()))
        })
        .unwrap_or_else(|| Ok(AttributionSettings::default()))
}

pub fn save(
    db: &Connection,
    params: &SaveAttributionSettingsParams,
) -> Result<AttributionSettings, BridgeError> {
    let normalized = params.settings.clone();
    db.execute(
        "INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at)
         VALUES(?1,?2,?3,?4,?4) ON CONFLICT(kind,id) DO UPDATE SET payload=excluded.payload,updated_at=excluded.updated_at",
        params![
            KIND,
            ID,
            serde_json::to_string(&normalized)
                .map_err(|error| BridgeError::Invalid(error.to_string()))?,
            chrono::Utc::now().to_rfc3339()
        ],
    )?;
    load(db)
}

pub fn hide_enabled(db: &Connection) -> bool {
    load(db).map(|settings| settings.hide_ai_attribution).unwrap_or(false)
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
    fn attribution_settings_defaults_to_visible() {
        let (_scratch, db) = db();
        let loaded = load(&db).unwrap();
        assert!(!loaded.hide_ai_attribution);
        assert!(!hide_enabled(&db));
    }

    #[test]
    fn attribution_settings_round_trips() {
        let (_scratch, db) = db();
        let on = save(
            &db,
            &SaveAttributionSettingsParams {
                settings: AttributionSettings { hide_ai_attribution: true },
            },
        )
        .unwrap();
        assert!(on.hide_ai_attribution);
        assert!(hide_enabled(&db));
        assert_eq!(load(&db).unwrap(), on);

        let off = save(
            &db,
            &SaveAttributionSettingsParams {
                settings: AttributionSettings { hide_ai_attribution: false },
            },
        )
        .unwrap();
        assert!(!off.hide_ai_attribution);
        assert!(!hide_enabled(&db));
    }
}

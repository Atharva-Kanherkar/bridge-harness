//! Whether search may call a model, and which one.

use bridge_protocol::messages::{ChatSearchSettings, SaveChatSearchSettingsParams};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};

use crate::BridgeError;

const KIND: &str = "chat_search_settings";
const ID: &str = "global";
const MAX_MODEL_CHARS: usize = 200;

/// The cheapest Claude model the adapter accepts by alias. Search spends a
/// few hundred tokens a query, so the smallest model is the right default.
pub const DEFAULT_MODEL: &str = "haiku";

pub fn load(db: &Connection) -> Result<ChatSearchSettings, BridgeError> {
    let payload: Option<String> = db
        .query_row(
            "SELECT payload FROM configuration_entries WHERE kind=?1 AND id=?2",
            params![KIND, ID],
            |row| row.get(0),
        )
        .optional()?;
    payload
        .map(|payload| serde_json::from_str(&payload).map_err(|error| BridgeError::Invalid(error.to_string())))
        .unwrap_or_else(|| Ok(ChatSearchSettings::default()))
}

pub fn save(db: &Connection, params: &SaveChatSearchSettingsParams) -> Result<ChatSearchSettings, BridgeError> {
    let model = params
        .settings
        .model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_owned);
    if model.as_ref().is_some_and(|model| model.chars().count() > MAX_MODEL_CHARS) {
        return Err(BridgeError::Invalid(format!(
            "A search model id must be at most {MAX_MODEL_CHARS} characters"
        )));
    }
    let settings = ChatSearchSettings {
        deep_search: params.settings.deep_search,
        model,
    };
    let payload = serde_json::to_string(&settings).map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let now = Utc::now().to_rfc3339();
    db.execute(
        "INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at) VALUES(?1,?2,?3,?4,?4)
         ON CONFLICT(kind,id) DO UPDATE SET payload=excluded.payload, updated_at=excluded.updated_at",
        params![KIND, ID, payload, now],
    )?;
    Ok(settings)
}

/// The model a deep search runs on.
pub fn model(settings: &ChatSearchSettings) -> String {
    settings.model.clone().unwrap_or_else(|| DEFAULT_MODEL.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::store::open(&dir.path().join("bridge.db")).unwrap();
        (dir, db)
    }

    #[test]
    fn defaults_to_deep_search_on_the_cheapest_model() {
        let (_dir, db) = db();
        let settings = load(&db).unwrap();
        assert!(settings.deep_search);
        assert_eq!(settings.model, None);
        assert_eq!(model(&settings), "haiku");
    }

    #[test]
    fn save_round_trips_and_blank_model_means_default() {
        let (_dir, db) = db();
        let saved = save(
            &db,
            &SaveChatSearchSettingsParams {
                settings: ChatSearchSettings {
                    deep_search: false,
                    model: Some("  sonnet ".into()),
                },
            },
        )
        .unwrap();
        assert_eq!(saved.model.as_deref(), Some("sonnet"));
        assert_eq!(load(&db).unwrap(), saved);
        let blank = save(
            &db,
            &SaveChatSearchSettingsParams {
                settings: ChatSearchSettings {
                    deep_search: true,
                    model: Some("   ".into()),
                },
            },
        )
        .unwrap();
        assert_eq!(blank.model, None);
        assert_eq!(load(&db).unwrap(), blank);
    }

    #[test]
    fn an_absurd_model_id_is_refused() {
        let (_dir, db) = db();
        let error = save(
            &db,
            &SaveChatSearchSettingsParams {
                settings: ChatSearchSettings {
                    deep_search: true,
                    model: Some("x".repeat(201)),
                },
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("at most"));
    }
}

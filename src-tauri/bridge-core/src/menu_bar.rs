//! Menu presentation preferences stored by the central backend.
use crate::BridgeError;
use bridge_protocol::messages::MenuBarSettings;
use rusqlite::{params, Connection, OptionalExtension};

pub fn load(db: &Connection) -> Result<MenuBarSettings, BridgeError> {
    let payload: Option<String> = db
        .query_row(
            "SELECT payload FROM configuration_entries WHERE kind='menu_bar' AND id='default'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let settings = payload
        .map(|p| {
            serde_json::from_str(&p)
                .map_err(|e| BridgeError::Invalid(format!("Invalid Menu Bar settings: {e}")))
        })
        .transpose()?
        .unwrap_or_default();
    validate(&settings)?;
    Ok(settings)
}

fn validate(settings: &MenuBarSettings) -> Result<(), BridgeError> {
    if settings.schema_version != 1 || ![0, 60, 300, 900, 1800].contains(&settings.refresh_seconds)
    {
        return Err(BridgeError::Invalid(
            "Unsupported Menu Bar settings version or refresh interval".into(),
        ));
    }
    Ok(())
}

pub fn save(db: &Connection, settings: &MenuBarSettings) -> Result<MenuBarSettings, BridgeError> {
    validate(settings)?;
    db.execute("INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at) VALUES('menu_bar','default',?1,?2,?2)
        ON CONFLICT(kind,id) DO UPDATE SET payload=excluded.payload,updated_at=excluded.updated_at",
        params![serde_json::to_string(settings).map_err(|e| BridgeError::Invalid(e.to_string()))?, chrono::Utc::now().to_rfc3339()])?;
    load(db)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persists_settings_and_rejects_invalid_changes_without_overwriting() {
        let temp = tempfile::tempdir().unwrap();
        let db = crate::store::open(&temp.path().join("test.db")).unwrap();
        let mut settings = load(&db).unwrap();
        settings.enabled = false;
        save(&db, &settings).unwrap();
        settings.refresh_seconds = 2;
        assert!(save(&db, &settings).is_err());
        assert!(!load(&db).unwrap().enabled);
        assert_eq!(load(&db).unwrap().refresh_seconds, 300);
    }
}
